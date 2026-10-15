#!/usr/bin/env bash
# Sparse-clone MPXJ's test data (LGPL-2.1, not redistributed in this repo) into ./corpus/mpxj.
set -euo pipefail
mkdir -p corpus
if [ ! -d corpus/mpxj ]; then
  git clone --depth 1 --filter=blob:none --sparse https://github.com/joniles/mpxj corpus/mpxj
fi
git -C corpus/mpxj sparse-checkout set junit/data
echo "corpus ready: corpus/mpxj/junit/data (MPXJ_DATA=$PWD/corpus/mpxj/junit/data)"
