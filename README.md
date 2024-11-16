# CanDo

CanDo (`cando`) is a command-line tool designed to simplify and optimize the representation of platform-specific, heavyweight executables within a conda environment. It does so by converting them into a lightweight, human-readable text file. This approach enables efficient storage of executables in source control without inflating repository size, paving the way to check toolchains and other build tools directly into the repo. As a result, CanDo minimizes dependencies on the host environment, promoting reproducible builds and reducing setup complexity.

This project takes inspiration from [dotslash](https://github.com/facebook/dotslash).

## Installation

You can build the application using cargo or just download a binary from the latest release and add the directory containing the binary in `PATH`.

## Usage

1. Create a conda environment with the required packages.

2. Use [conda-lock](https://github.com/conda/conda-lock) to create a conda lockfile for the enviroment.

3. Create a cando file. A cando file is a simple executable yaml file which calls cando executable and provides it with the necessary information to get the lockfile and create a cache. The name of executable to be run within the conda environment should match the name of the cando file itself.

A cando file from example containing information for running rg

```yaml
#!/usr/bin/env cando 

# Path relative to the directory in which this cando file exists
lockfile: ../con_exec/src/env-lock.yaml
# Optional absolute path to the cache directory which cando should use
# it defaults to using XDG_CACHE_HOME/HOME env vars for getting the cache directory
# cache: /Users/satyajeet/Desktop/code/con_exec/target/mycache
```

4. You can also generate cando executables using `cando generate`

The protoc file generated in examples dir was done using:

```bash
cando generate --bin protoc --lockfile example/my_env/env-lock.yaml --output ./example/
```

An example of self-sufficient cando executable:

```yaml
#!/usr/bin/env cando

inlined_conda_pkgs:
  hash: 7ca536558f6b8bfd9d39a2d7ff2d21b95fa71e4f9bd59d318a10ce71eb892394
  conda_pkgs:
  - name: ca-certificates-2024.8.30-hf0a4a13_0.conda
    url: https://conda.anaconda.org/conda-forge/osx-arm64/ca-certificates-2024.8.30-hf0a4a13_0.conda
    sha256: 2db1733f4b644575dbbdd7994a8f338e6ef937f5ebdb74acd557e9dda0211709
  - name: libabseil-20240722.0-cxx17_hf9b8971_1.conda
    url: https://conda.anaconda.org/conda-forge/osx-arm64/libabseil-20240722.0-cxx17_hf9b8971_1.conda
    sha256: 90bf08a75506dfcf28a70977da8ab050bcf594cd02abd3a9d84a22c9e8161724
  - name: libcxx-19.1.2-ha82da77_0.conda
    url: https://conda.anaconda.org/conda-forge/osx-arm64/libcxx-19.1.2-ha82da77_0.conda
    sha256: 9c714110264f4fe824d40e11ad39b0eda65251f87826c81f4d67ccf8a3348d29
  - name: libprotobuf-5.28.3-h8f0b736_0.conda
    url: https://conda.anaconda.org/conda-forge/osx-arm64/libprotobuf-5.28.3-h8f0b736_0.conda
    sha256: d95a239216db16ff5cac10be45c11afd2b1bb5dd17c9f3cabb35c6dd2f2f13fd
  - name: libzlib-1.3.1-h8359307_2.conda
    url: https://conda.anaconda.org/conda-forge/osx-arm64/libzlib-1.3.1-h8359307_2.conda
    sha256: ce34669eadaba351cd54910743e6a2261b67009624dbc7daeeafdef93616711b
  - name: openssl-3.3.2-h8359307_0.conda
    url: https://conda.anaconda.org/conda-forge/osx-arm64/openssl-3.3.2-h8359307_0.conda
    sha256: 940fa01c4dc6152158fe8943e05e55a1544cab639df0994e3b35937839e4f4d1
  - name: ripgrep-14.1.1-h0ef69ab_0.conda
    url: https://conda.anaconda.org/conda-forge/osx-arm64/ripgrep-14.1.1-h0ef69ab_0.conda
    sha256: bea65d7f355ac3db84b046e2db3b203d78ac261451bf5dd7a5719fc8102fa73e
```

PRO-TIP: You can symlink the same cando file to different executables contained in the conda environment. That is if protoc and rg were in the same conda environment you can simple run cando generate once to create a cando executable and symlink protoc and rg to that and it'd work as expected.

*Debug Mode:* In case of issues you can use cando in debug mode by setting the env var `CANDO_DEBUG=1`.

## Self contained python (possibly anything) scripts

You can look at the script in `example/standalone_py_script/script.py` the shebang in this script uses the python binary contained inside this directory itself i.e. `example/standalone_py_script/python` which is a cando binary. This cando binary has a conda environment which contains all packges like matplotlib, scipy, numpy which our script uses.

## What's with the name `cando`?

Its just the word conda with a and o swapped which makes sense in the context as well.

## Why CanDo?

Without cando a normal workflow of using binaries from conda can look something like:

```bash

micromamba create -n my_env -f some_env.yaml -y
micromamba activate my_env
## Run you binary contained in the env
```

As you can see this is much more tedious then just running the binary directly. There are several other problems which I've faced as well. Lets assume you want to run 2 binaries and their packages are incompatible ie you can't install their pkgs in one env, in this can you'd have to create 2 different envs and then either activate them one by one or use binaries from the env manually which is tedious. There are many such issues on similar lines and cando helps us solve them.


## Design Notes

- CanDo is NOT a package manager its just a simple tool which helps to run binaries from conda envs seamlessly.

- CanDo doesn't depend on any package manager in conda ecosystem (conda/micromamba/mamba/pixi etc), this helps us in making cando a bootstrapping endpoint in our system, ie if we somehow have cando then we can distribute anything from conda with it without needing any pkg manager.

- CanDo doesn't concern itself about performing cleanup etc, they're user's responsibility. They can add cron-jobs or use something like [dir_ttl](https://github.com/satyajeet104/dir_ttl).


Compatibility:

I haven't tested it yet on Windows, I have tested it on mac, linux and that too not rigorously so there might be some rough edges.
