# CanDo

CanDo (`cando`) is a command-line tool designed to simplify and optimize the representation of platform-specific, heavyweight executables within a conda environment. It does so by converting them into a lightweight, human-readable text file. This approach enables efficient storage of executables in source control without inflating repository size, paving the way to check toolchains and other build tools directly into the repo. As a result, CanDo minimizes dependencies on the host environment, promoting reproducible builds and reducing setup complexity.

This project takes inspiration from [dotslash](https://github.com/facebook/dotslash).

## Installation
TODO (create a GH release, currently just do cargo build and add the the build path to $PATH)

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
