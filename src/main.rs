use anyhow::anyhow;
use clap::Parser;
use itertools::{Itertools, Tee};
use rattler_conda_types::Platform;
use rattler_lock::LockFile;
use serde::{Deserialize, Serialize};
use std::env::ArgsOs;
use std::io::{Cursor, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tokio_retry::strategy::ExponentialBackoff;
use tracing::{debug, Level};

// RX for both owner and group
const DEFAULT_FILE_PERMISSIONS: u32 = 0o550;
const LOCK_FILE_NAME: &str = "__CANDO_LOCK__";
const SUCCESS_MARKER_NAME: &str = "__CANDO_SUCCESS_MARKER__";
// TODO(SS): Make them configurable?
const ENV_DIR_NAME: &str = "__ENV__";
const EXPONENTIAL_BACKOFF_BASE_MILLIS: u64 = 100;
const NUM_RETRIES: usize = 5;
const DEFAULT_CACHE_NAME: &str = ".cando_cache";
const DEFAULT_CACHE_ENV_DIRS: [&str; 2] = ["XDG_CACHE_HOME", "HOME"];

#[derive(Debug, Deserialize, Serialize)]
struct InlinedCondaPkgs {
    /// Inlined data about conda pkgs needed for the current particular platform.
    hash: String,
    conda_pkgs: Vec<CondaPkg>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Schema {
    /// Relative path to lockfile
    #[serde(skip_serializing_if = "Option::is_none")]
    lockfile_path: Option<PathBuf>,
    /// Optinally cache path can be defined, default path is $HOME/.cando_cache
    #[serde(skip_serializing_if = "Option::is_none")]
    cache: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inlined_conda_pkgs: Option<InlinedCondaPkgs>,
}

impl Schema {
    fn validate_schema(&self) -> anyhow::Result<()> {
        if self.lockfile_path.is_none() && self.inlined_conda_pkgs.is_none() {
            Err(anyhow!(
                "`lockfile_path` and `inlined_conda_pkgs` can't be None at the same time please specify atleast one of them"
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
struct CondaPkg {
    name: String,
    url: url::Url,
    sha256: String,
}

#[derive(Parser, Debug)]
struct GenerateOpts {
    #[clap(
        long,
        short,
        help = "Executable for which cando file is to be generated"
    )]
    bin: String,
    #[clap(
        long,
        short,
        default_value = "true",
        help = "If set to false then it cando file depends on external lockfiles for execution"
    )]
    inline: bool,
    #[clap(
        long,
        short,
        default_value = ".",
        help = "Output directory for the cando executable"
    )]
    output: PathBuf,
    #[clap(long, short, help = "lockfile to be used in cando exe generation")]
    lockfile: PathBuf,
    #[clap(long, short, help = "Cache dir to be used by cando")]
    cache: Option<PathBuf>,
}

#[derive(Parser, Debug)]
enum Commands {
    Generate(GenerateOpts),
}

#[derive(Parser, Debug)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

async fn download_pkg(
    pkg: &CondaPkg,
    client: reqwest::Client,
    download_path: &Path,
) -> anyhow::Result<PathBuf> {
    debug!("downloading {pkg:?}");
    let file_path = download_path.join(&pkg.name);
    let extracted_pkg = file_path.with_extension("");

    // Perform cleanup in case its a rerun, ignore any errors during cleanup
    let _ = tokio::fs::remove_file(&file_path).await;
    let _ = tokio::fs::remove_dir_all(&extracted_pkg).await;

    let url = pkg.url.clone();
    let mut file = tokio::fs::File::create(&file_path).await?;
    let bytes = client.get(url).send().await?.bytes().await?;
    let mut content = Cursor::new(bytes);
    tokio::io::copy(&mut content, &mut file).await?;

    let res = rattler_package_streaming::tokio::fs::extract(&file_path, &extracted_pkg).await?;
    let actual_sha = format!("{:x}", res.sha256);
    let expected_sha = &pkg.sha256;

    if *expected_sha != actual_sha {
        Err(anyhow!(
            "SHA mismatch actual {expected_sha} expected {actual_sha}"
        ))
    } else {
        Ok(extracted_pkg)
    }
}

fn hash_conda_pkgs(pkgs: &[CondaPkg]) -> String {
    // Hash all the pkgs to create a unique hash for the env, first sort the list
    // so hashes won't change due to changes in ordering in the lockfile
    let mut pkgs = pkgs.to_owned();
    pkgs.sort_by(|a, b| a.name.cmp(&b.name));
    let accum_hash = pkgs
        .iter()
        .fold("".to_owned(), |acc, c| acc + c.sha256.as_str());
    sha256::digest(accum_hash)
}

fn set_permissions_on_path(p: &Path, mask: u32) -> std::io::Result<()> {
    let mut perms = std::fs::metadata(p)?.permissions();
    perms.set_mode(mask);
    std::fs::set_permissions(p, perms)
}

async fn install_env(pkgs: Vec<CondaPkg>, install_dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(install_dir)?;
    let file_lock_path = install_dir.join(LOCK_FILE_NAME);
    debug!("Waiting until we can aquire the file lock");
    let _lock_guard = named_lock::NamedLock::with_path(file_lock_path)?.lock()?;
    debug!("File Lock Acquired");

    let env_path = install_dir.join(ENV_DIR_NAME);
    let success_marker = env_path.join(SUCCESS_MARKER_NAME);
    if success_marker.try_exists()? {
        return Ok(env_path);
    }

    let download_path = install_dir.join("downloads");
    std::fs::create_dir_all(&download_path)?;

    let client = reqwest::Client::new();
    let retry_strategy =
        ExponentialBackoff::from_millis(EXPONENTIAL_BACKOFF_BASE_MILLIS).take(NUM_RETRIES);

    let extracted_pkgs = futures::future::join_all(pkgs.iter().map(|c| {
        tokio_retry::Retry::spawn(retry_strategy.clone(), || {
            download_pkg(c, client.clone(), &download_path)
        })
    }))
    .await
    .into_iter()
    .collect::<anyhow::Result<Vec<_>>>()?;

    std::fs::create_dir_all(&env_path)?;
    for extracted_pkg in extracted_pkgs {
        rattler::install::link_package(
            &extracted_pkg,
            &env_path,
            &Default::default(),
            Default::default(),
        )
        .await?;
    }

    std::fs::remove_dir_all(download_path)?;
    std::fs::File::create(success_marker)?;
    // Set RX perms for owner/group
    set_permissions_on_path(&env_path, DEFAULT_FILE_PERMISSIONS)?;

    Ok(env_path)
}

fn get_conda_pkgs_from_lockfile(lockfile_path: &Path) -> anyhow::Result<Vec<CondaPkg>> {
    let lockfile = LockFile::from_path(lockfile_path)?;
    let platform = Platform::current();
    if lockfile.environments().len() > 1 {
        return Err(anyhow!(
            "More than one env in the lockfile are not supported"
        ));
    }

    let pkgs = lockfile
        .environments()
        .next()
        .ok_or(anyhow!("No environment found in the lockfile"))?
        .1
        .conda_repodata_records()?
        .get(&platform)
        .ok_or(anyhow!("Platform `{platform:?}` not found"))?
        .iter()
        .map(|r| CondaPkg {
            name: r.file_name.clone(),
            url: r.url.clone(),
            sha256: format!("{:x}", r.package_record.sha256.unwrap()),
        })
        .collect::<Vec<_>>();

    if pkgs.is_empty() {
        Err(anyhow!(
            "No packages found for the current platform {platform:#?} in the lockfile"
        ))
    } else {
        Ok(pkgs)
    }
}

fn get_info_from_cando_file(cando_file_path: &Path) -> anyhow::Result<(PathBuf, Vec<CondaPkg>)> {
    let cando_schema: Schema = {
        let content = std::fs::read_to_string(cando_file_path)?;
        serde_yaml::from_str(&content)?
    };

    cando_schema.validate_schema()?;

    let Schema {
        lockfile_path,
        cache,
        inlined_conda_pkgs,
    } = cando_schema;

    let (hash, conda_pkgs) = match inlined_conda_pkgs {
        Some(InlinedCondaPkgs { hash, conda_pkgs }) => (hash, conda_pkgs),
        None => {
            let lockfile_abs_path = {
                let mut cando_file = PathBuf::from(cando_file_path);
                // remove the cando exe name itself from the path
                if !cando_file.pop() {
                    return Err(anyhow!("Invalid file exec path {cando_file:#?}"));
                }
                // unwrap is safe here as we've already validated the schema above and its guaranteed that
                // lockfil_path is not none
                cando_file.join(lockfile_path.unwrap()).canonicalize()?
            };
            let conda_pkgs = get_conda_pkgs_from_lockfile(&lockfile_abs_path)?;
            (hash_conda_pkgs(&conda_pkgs), conda_pkgs)
        }
    };

    let cache_dir = match cache {
        Some(cache_path) => cache_path,
        None => {
            // We prefer XDG_CACHE_HOME but fallback to HOME if it isn't set (like in macos).
            let cache_env_dir = DEFAULT_CACHE_ENV_DIRS
                .iter()
                .filter_map(std::env::var_os)
                .next()
                .ok_or(anyhow!(
                    "XDG_CACHE_HOME/HOME env var unset, consider setting one 
                of them or set the cachedir param in cando file"
                ))?;

            PathBuf::from(cache_env_dir).join(DEFAULT_CACHE_NAME)
        }
    };

    let cache_path = cache_dir.join(hash);

    Ok((cache_path, conda_pkgs))
}

async fn get_exe_from_cando_file(cando_file_path: &Path) -> anyhow::Result<PathBuf> {
    if !cando_file_path.try_exists()? {
        return Err(anyhow!("Cando file {cando_file_path:#?} doesn't exist"));
    }
    let (cache_path, conda_pkgs) = get_info_from_cando_file(cando_file_path)?;
    let env_path = install_env(conda_pkgs, &cache_path).await?;
    let exe_name = cando_file_path
        .file_name()
        .ok_or(anyhow!("Invalid cando filename {cando_file_path:#?}"))?;
    let exe_path = env_path.join("bin").join(exe_name);
    if !exe_path.try_exists()? {
        Err(anyhow!(
            "Executable {exe_name:#?} doesn't exist in the given env,
            rename the cando file and set it to the correct exe name"
        ))
    } else {
        Ok(exe_path)
    }
}

async fn try_exec_with_args(mut exec_args: Tee<ArgsOs>) -> anyhow::Result<()> {
    if let Some(file_arg) = exec_args.nth(1) {
        let exe = get_exe_from_cando_file(Path::new(&file_arg)).await?;
        debug!("executable path: {exe:#?}");
        let mut command = std::process::Command::new(exe);
        command.args(exec_args);
        std::os::unix::process::CommandExt::arg0(&mut command, file_arg);
        let exec_error = std::os::unix::process::CommandExt::exec(&mut command);
        Err(anyhow!(
            "Unable to call exec the binary, error: {exec_error:#?}"
        ))
    } else {
        Ok(())
    }
}

fn handle_cli(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Generate(GenerateOpts {
            bin,
            inline,
            output,
            lockfile,
            cache,
        }) => {
            let conda_pkgs = get_conda_pkgs_from_lockfile(&lockfile)?;
            let hash = hash_conda_pkgs(&conda_pkgs);
            let lockfile_path = {
                if inline {
                    None
                } else {
                    Some(lockfile)
                }
            };
            let inlined_conda_pkgs = {
                if inline {
                    Some(InlinedCondaPkgs { hash, conda_pkgs })
                } else {
                    None
                }
            };
            let cando_file = Schema {
                lockfile_path,
                cache,
                inlined_conda_pkgs,
            };
            let cando_file_str = format!(
                "#!/usr/bin/env cando\n\n{}",
                serde_yaml::to_string(&cando_file)?
            );
            let output_file_path = output.join(bin);
            let mut output_file = std::fs::File::create(&output_file_path)?;
            output_file.write_all(cando_file_str.as_bytes())?;
            set_permissions_on_path(&output_file_path, DEFAULT_FILE_PERMISSIONS)?;
        }
    }
    Ok(())
}

async fn run_cando(args: ArgsOs) -> anyhow::Result<()> {
    let (exec_args, clap_args) = args.into_iter().tee();
    match try_exec_with_args(exec_args).await {
        Err(exec_error) => {
            debug!("Exec failed with error: {exec_error} trying to run cli instead");
            match Cli::try_parse_from(clap_args) {
                Ok(cli) => handle_cli(cli),
                Err(cli_error) => Err(anyhow!(
                    "`{cli_error}` \n and exec'ing failed due to: `{exec_error:#?}`"
                )),
            }
        }
        Ok(_) => {
            debug!("Cando (exec) execution successful");
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let debug_level = match std::env::var("CANDO_DEBUG") {
        Ok(v) => match v.as_str() {
            "1" => Level::DEBUG,
            _ => Level::WARN,
        },
        _ => Level::WARN,
    };

    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_max_level(debug_level)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    let args = std::env::args_os();
    run_cando(args).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_schema() {
        let valid_schema = Schema {
            lockfile_path: Some("test_lockfile".into()),
            cache: None,
            inlined_conda_pkgs: None,
        };
        assert!(
            valid_schema.validate_schema().is_ok(),
            "Schema validation should succeed"
        );

        let invalid_schema = Schema {
            lockfile_path: None,
            cache: None,
            inlined_conda_pkgs: None,
        };
        assert!(
            invalid_schema.validate_schema().is_err(),
            "Schema validation should fail"
        );
    }

    #[test]
    fn misc_test() {
        let lockfile_path = Path::new("example/my_env/env-lock.yaml");
        let pkgs = get_conda_pkgs_from_lockfile(&lockfile_path).unwrap();
        let exp_hash = "7ca536558f6b8bfd9d39a2d7ff2d21b95fa71e4f9bd59d318a10ce71eb892394";
        // hashing function checks
        assert_eq!(hash_conda_pkgs(&pkgs), exp_hash);
        // Test for file with relative lockfiles
        let rg = Path::new("example/rg");
        let (cache_path, cp) = get_info_from_cando_file(rg).unwrap();
        assert_eq!(cache_path.file_name().unwrap().to_str().unwrap(), exp_hash);
        assert_eq!(cp, pkgs);
        // Test for file with inlined pkgs
        let protoc = Path::new("example/protoc");
        let (cache_path, cp) = get_info_from_cando_file(protoc).unwrap();
        assert_eq!(cache_path.file_name().unwrap().to_str().unwrap(), exp_hash);
        assert_eq!(cp, pkgs);
    }
}

// TODO(SS): Add integration tests as well
