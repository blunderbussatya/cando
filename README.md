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
cache: /Users/satyajeet/Desktop/code/con_exec/target/mycache
```

4. You can also generate cando executables using `cando generate`

The protoc file generated in examples dir was done using:
```bash
cando generate --bin protoc --lockfile example/my_env/env-lock.yaml --output ./example/
```
TODO(SS): Add more detailed docs

Debug Mode: In case of issues you can use cando in debug mode by setting the env var `CANDO_DEBUG=1`.

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
