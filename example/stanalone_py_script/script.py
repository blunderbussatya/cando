#!/bin/sh
"exec" "`dirname $0`/python" "$0" "$@"

# The python interpreter comes from cando's version
# we can now use all the libs which we added in our env

import numpy
import matplotlib
import scipy
