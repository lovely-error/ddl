# GitHub CI/CD & Multi-Platform Release Pipeline

This document explains how automated testing, multi-platform binary compilation, and release publishing are configured and controlled for DDL.

---

## Table of Contents

- [Overview](#overview)
- [Workflow Architecture](#workflow-architecture)
- [Continuous Integration (`ci.yml`)](#continuous-integration-ciyml)
  - [Triggers](#triggers)
  - [Validation Steps](#validation-steps)
- [Multi-Platform Release Pipeline (`release.yml`)](#multi-platform-release-pipeline-releaseyml)
  - [Supported Target Architectures](#supported-target-architectures)
  - [Trigger Modes](#trigger-modes)
  - [Checksums & Release Assets](#checksums--release-assets)
- [Operational Runbook](#operational-runbook)
  - [1. Publishing an Official Versioned Release](#1-publishing-an-official-versioned-release)
  - [2. Controlling Nightly Builds](#2-controlling-nightly-builds)
  - [3. Triggering a Manual Build via GitHub UI](#3-triggering-a-manual-build-via-github-ui)
- [Repository Permissions & Setup](#repository-permissions--setup)
- [User Installation Instructions](#user-installation-instructions)

---

## Overview

The DDL repository includes automated GitHub Actions pipelines located in [`.github/workflows/`](../.github/workflows):

1. **`ci.yml`**: Runs comprehensive unit, integration, clippy, and formatting checks on pull requests and pushes to `master`.
2. **`release.yml`**: Compiles fully optimized, standalone binaries for **Linux (x86_64, ARM64)**, **Windows (x64)**, and **macOS (Apple Silicon, Intel)**, generating cryptographic checksums and publishing them to GitHub Releases.

---

## Workflow Architecture

```
                                    ┌───────────────────────┐
                                    │ Git Push / PR / Event │
                                    └───────────┬───────────┘
                                                │
                 ┌──────────────────────────────┴──────────────────────────────┐
                 ▼                                                             ▼
    ┌─────────────────────────┐                                   ┌─────────────────────────┐
    │     ci.yml (Testing)    │                                   │   release.yml (Release) │
    ├─────────────────────────┤                                   ├─────────────────────────┤
    │ • cargo test (Linux)    │                                   │ Triggers:               │
    │ • cargo test (Windows)  │                                   │ • Tag push: v*          │
    │ • cargo test (macOS)    │                                   │ • Nightly cron (02:00)  │
    │ • clippy --all-targets  │                                   │ • workflow_dispatch     │
    │ • ddl fmt --check       │                                   └────────────┬────────────┘
    └─────────────────────────┘                                                │
                                                                               ▼
                                                   ┌───────────────────────────────────────────────┐
                                                   │ Build Matrix (5 Platforms):                   │
                                                   │ • Linux x86_64 (static musl)                  │
                                                   │ • Linux aarch64 (static musl via cross)       │
                                                   │ • Windows x86_64 (MSVC)                       │
                                                   │ • macOS Apple Silicon (M1-M4)                 │
                                                   │ • macOS Intel (x86_64)                        │
                                                   └───────────────────────┬───────────────────────┘
                                                                           │
                                                                           ▼
                                                   ┌───────────────────────────────────────────────┐
                                                   │ Packaging & Verification:                     │
                                                   │ • Archive binaries (.tar.gz / .zip)           │
                                                   │ • Generate SHA256SUMS.txt                     │
                                                   │ • Publish assets to GitHub Releases           │
                                                   └───────────────────────────────────────────────┘
```

---

## Continuous Integration (`ci.yml`)

Located at [`.github/workflows/ci.yml`](../.github/workflows/ci.yml).

### Triggers
- Any push to `master` or `main`.
- Any pull request targeting `master` or `main`.

### Validation Steps
- **Cross-Platform Test Matrix**: Executes `cargo test` concurrently on `ubuntu-latest`, `windows-latest`, and `macos-latest`.
- **Clippy Linting**: Runs `cargo clippy --all-targets -- -D warnings` on the pinned nightly toolchain (`rust-toolchain.toml`).
- **Formatting Hygiene**: Runs `ddl fmt --check examples/*.ddl` to guarantee that checked-in examples maintain formatting without AST drift.

---

## Multi-Platform Release Pipeline (`release.yml`)

Located at [`.github/workflows/release.yml`](../.github/workflows/release.yml).

### Supported Target Architectures

| Target Platform | GitHub Runner | Rust Target | Archive Format | Technical Characteristics |
|---|---|---|---|---|
| **Linux x86_64** | `ubuntu-latest` | `x86_64-unknown-linux-musl` | `.tar.gz` | **Statically linked** with musl-libc. Zero system dependencies; runs on any Linux distro (Ubuntu, Debian, Alpine, RHEL, Arch). |
| **Linux ARM64** | `ubuntu-latest` | `aarch64-unknown-linux-musl` | `.tar.gz` | Cross-compiled using `cross`. Runs on Raspberry Pi 4/5, AWS Graviton, Ampere servers. |
| **Windows x86_64** | `windows-latest` | `x86_64-pc-windows-msvc` | `.zip` | Standalone `ddl.exe`. |
| **macOS Apple Silicon** | `macos-14` | `aarch64-apple-darwin` | `.tar.gz` | Native ARM64 binary for M1, M2, M3, and M4 Macs. |
| **macOS Intel** | `macos-13` | `x86_64-apple-darwin` | `.tar.gz` | Native x86_64 binary for older Intel Macs. |

### Trigger Modes

The pipeline operates in two modes depending on how it is triggered:

1. **Versioned Release Mode (Triggered by Git Tag)**:
   - Triggered when pushing a tag starting with `v` (e.g. `v0.1.0`).
   - Generates release named `DDL v0.1.0`.
   - Marked as an official release (`prerelease: false`, `make_latest: true`).
   - Automatically generates release notes from merged pull requests.
2. **Nightly Pre-Release Mode (Triggered by Cron or Manual Dispatch)**:
   - Triggered automatically every night at **02:00 UTC** (`cron: '0 2 * * *'`).
   - Generates release named `DDL Nightly Build (YYYY-MM-DD)`.
   - Marked as a pre-release (`prerelease: true`, `make_latest: false`).
   - Updates the floating **`nightly`** tag on GitHub, overwriting older binaries so users always have access to the latest development build.

### Checksums & Release Assets

Every release includes:
- The archived binaries: `ddl-<tag>-<target>.tar.gz` (or `.zip`).
- Embedded documentation (`README.md`, `desc.md`, `LICENSE`).
- **`SHA256SUMS.txt`**: A manifest of SHA-256 cryptographic hashes for all packages, enabling automated integrity verification.

---

## Operational Runbook

### 1. Publishing an Official Versioned Release

To publish a formal release:

1. Update the version in [`Cargo.toml`](../Cargo.toml):
   ```toml
   [package]
   name = "ddl"
   version = "0.2.0"
   ```
2. Commit and push to `master`:
   ```bash
   git commit -am "chore: bump version to 0.2.0"
   git push origin master
   ```
3. Tag the commit and push the tag:
   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```
4. GitHub Actions will automatically:
   - Build all 5 targets.
   - Package archives and generate `SHA256SUMS.txt`.
   - Publish the `v0.2.0` release under your repository's **Releases** page.

---

### 2. Controlling Nightly Builds

- **Schedule Adjustment**: The nightly schedule is controlled by the cron line in `.github/workflows/release.yml`:
  ```yaml
  schedule:
    - cron: '0 2 * * *'  # Runs at 02:00 UTC daily
  ```
- **Disabling Nightly Builds**: To pause automated nightly builds, simply comment out the `schedule` block in `.github/workflows/release.yml`.

---

### 3. Triggering a Manual Build via GitHub UI

You can build and publish binaries on demand without creating a git commit:

1. Navigate to your repository on GitHub.
2. Click the **Actions** tab.
3. Select the **Release** workflow in the left sidebar.
4. Click **Run workflow**:
   - Choose the branch (default: `master`).
   - Optionally check `Build as nightly pre-release`.
   - Click the green **Run workflow** button.

---

## Repository Permissions & Setup

For the release workflow to publish binaries to your GitHub Releases page, GitHub Actions needs write permissions:

1. In your GitHub repository, navigate to **Settings** $\rightarrow$ **Actions** $\rightarrow$ **General**.
2. Scroll to **Workflow permissions**.
3. Select **Read and write permissions**.
4. Check **Allow GitHub Actions to create and approve pull requests** (if applicable).
5. Click **Save**.

*(Note: `.github/workflows/release.yml` already includes `permissions: contents: write`, which ensures full access when this setting is enabled).*

---

## User Installation Instructions

You can include these download instructions in your repository's download page or release notes:

### Linux / macOS (Direct Download)

```bash
# Example: Linux x86_64
curl -LO https://github.com/<OWNER>/ddl/releases/latest/download/ddl-v0.1.0-x86_64-unknown-linux-musl.tar.gz
tar -xzf ddl-v0.1.0-x86_64-unknown-linux-musl.tar.gz
sudo mv ddl-*/ddl /usr/local/bin/
ddl --version
```

### Windows (PowerShell)

```powershell
# Example: Windows x86_64
Invoke-WebRequest -Uri "https://github.com/<OWNER>/ddl/releases/latest/download/ddl-v0.1.0-x86_64-pc-windows-msvc.zip" -OutFile "ddl.zip"
Expand-Archive -Path "ddl.zip" -DestinationPath "$HOME\bin"
# Add $HOME\bin to your PATH
ddl --version
```
