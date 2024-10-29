use anyhow::{anyhow, Result};
use rattler_conda_types::Platform;
use rattler_lock::LockFile;
use serde::Deserialize;
use std::env::ArgsOs;
use std::io::Cursor;
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

#[derive(Debug, Deserialize)]
struct Schema {
    /// Relative path to lockfile
    lockfile: PathBuf,
    /// Optinally cache path can be defined, default path is $HOME/.cando_cache
    cache: Option<PathBuf>,
}

#[derive(Debug)]
struct CondaPkg {
    name: String,
    url: url::Url,
    sha256: String,
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
    // Hash all the pkgs to create a unique hash for the env, `get_conda_pkgs_from_lockfile` also sorts the list
    // so hashes won't change due to changes in ordering in the lockfile
    let accum_hash = pkgs
        .iter()
        .fold("".to_owned(), |acc, c| acc + c.sha256.as_str());
    sha256::digest(accum_hash)
}

async fn install_env(pkgs: Vec<CondaPkg>, cache_path: &Path) -> anyhow::Result<PathBuf> {
    let install_dir = cache_path.join(hash_conda_pkgs(&pkgs));
    std::fs::create_dir_all(&install_dir)?;
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
    .collect::<Result<Vec<_>>>()?;

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
    let mut perms = std::fs::metadata(&env_path)?.permissions();
    perms.set_mode(DEFAULT_FILE_PERMISSIONS);
    tokio::fs::set_permissions(&env_path, perms).await?;

    Ok(env_path)
}

fn get_conda_pkgs_from_lockfile(
    lockfile: LockFile,
    platform: Platform,
) -> anyhow::Result<Vec<CondaPkg>> {
    if lockfile.environments().len() > 1 {
        return Err(anyhow!(
            "More than one env in the lockfile are not supported"
        ));
    }

    let (_, env) = lockfile
        .environments()
        .next()
        .ok_or(anyhow!("No environment found in the lockfile"))?;

    let env = env.conda_repodata_records()?;

    let mut pkgs = env
        .get(&platform)
        .ok_or(anyhow!("Platform not found"))?
        .iter()
        .map(|r| CondaPkg {
            name: r.file_name.clone(),
            url: r.url.clone(),
            sha256: format!("{:x}", r.package_record.sha256.unwrap()),
        })
        .collect::<Vec<_>>();
    if pkgs.is_empty() {
        return Err(anyhow!(
            "No packages found for the current platform {platform:#?} in the lockfile"
        ));
    }
    pkgs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(pkgs)
}

fn get_info_from_cando_file(cando_file_path: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    let Schema { lockfile, cache }: Schema = {
        let content = std::fs::read_to_string(cando_file_path)?;
        serde_yaml::from_str(&content)?
    };

    let lockfile_path = {
        let mut cando_file = PathBuf::from(cando_file_path);
        // remove the cando exe name itself from the path
        if !cando_file.pop() {
            return Err(anyhow!("Invalid file exec path {cando_file:#?}"));
        }
        cando_file.join(lockfile).canonicalize()?
    };

    let cache_path = match cache {
        Some(cache_path) => cache_path,
        None => {
            let home = std::env::var_os("HOME").ok_or(anyhow!(
                "HOME env var unset, consider setting it or set the cachedir param in cando file"
            ))?;
            PathBuf::from(home).join(DEFAULT_CACHE_NAME)
        }
    };

    Ok((lockfile_path, cache_path))
}

async fn run_cando(mut args: ArgsOs) -> anyhow::Result<()> {
    if let Some(file_arg) = args.nth(1) {
        let cando_file_path = PathBuf::from(&file_arg);
        let (lockfile_path, cache_path) = get_info_from_cando_file(&cando_file_path)?;
        let conda_pkgs = get_conda_pkgs_from_lockfile(
            LockFile::from_path(&lockfile_path)?,
            Platform::current(),
        )?;

        let exe_path = {
            let env_path = install_env(conda_pkgs, &cache_path).await?;
            let exe_name = cando_file_path
                .file_name()
                .ok_or(anyhow!("Invalid cando filename {cando_file_path:#?}"))?;
            let exe_path = env_path.join("bin").join(exe_name);
            if !exe_path.try_exists()? {
                return Err(anyhow!(
                    "Executable {exe_name:#?} doesn't exist in the given env,
                    rename the cando file and set it to the correct exe name"
                ));
            }
            exe_path
        };

        let mut command = std::process::Command::new(exe_path);
        command.args(args);
        std::os::unix::process::CommandExt::arg0(&mut command, file_arg);
        let exec_error = std::os::unix::process::CommandExt::exec(&mut command);
        return Err(anyhow!(
            "Unable to call exec the binary, error: {exec_error:#?}"
        ));
    }

    Ok(())
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

// TODO(SS): Add tests
