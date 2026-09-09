#!/usr/bin/env bash
# Installs the Verilator that tests/lint.rs is calibrated against, into
# /opt/verilator.
#
# Usage: bash .github/install-verilator.sh
#
# Invoked through `bash` rather than executed, like examples/verify.sh: this
# repository is developed on Windows, where the executable bit does not survive
# a checkout, so a script that depends on it fails in CI with "Permission
# denied" and nothing else.
#
# Built from source rather than taken from apt. Ubuntu ships 5.020 (from
# 2024-01-01), and examples/lint.vlt waives rules that did not exist then --
# MODMISSING arrived in 5.038 -- so a distro Verilator errors on the WAIVER
# FILE and never reads a line of the generated Verilog. That failure looks
# like sixteen lint failures and is really one version mismatch.
#
# Shared by the CI and release workflows so the two cannot drift apart.
set -euo pipefail

VERILATOR_VERSION="${VERILATOR_VERSION:-v5.046}"
PREFIX="${PREFIX:-/opt/verilator}"

if [ -x "$PREFIX/bin/verilator" ]; then
    echo "verilator already present at $PREFIX"
    "$PREFIX/bin/verilator" --version
    exit 0
fi

sudo apt-get update
sudo apt-get install -y \
    git help2man perl python3 make autoconf g++ flex bison ccache \
    libgoogle-perftools-dev numactl perl-doc zlib1g zlib1g-dev
# Ubuntu-only, and named differently across releases. Absence is not fatal.
sudo apt-get install -y libfl2 libfl-dev || true

git clone --depth 1 --branch "$VERILATOR_VERSION" \
    https://github.com/verilator/verilator /tmp/verilator-src
cd /tmp/verilator-src
autoconf
./configure --prefix="$PREFIX"
make -j"$(nproc)"
sudo make install

"$PREFIX/bin/verilator" --version
