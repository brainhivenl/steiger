# Steiger

A container build orchestrator for multi-service projects with native support for Bazel, Docker BuildKit, and Ko. Steiger coordinates parallel builds and handles registry operations with automatic platform detection.

## Project Status

- ✅ **Basic building and pushing**: Core functionality is stable
- ✅ **Multi-service parallel builds**: Working with real-time progress
- ✅ **Registry integration**: Push to any OCI-compliant registry
- ⏳ **Dev mode**: File watching and rebuild-on-change (planned)
- 🚧 **Deploy**: Native Kubernetes deployment support (works, but needs to be extended)

## Supported Builders

### Docker BuildKit

Uses [Docker BuildKit](https://docs.docker.com/build/buildkit/) with the `docker-container` driver for efficient, cached builds. Steiger manages the BuildKit builder instance automatically.

Requirements:

- Docker with BuildKit support
- `docker-container` driver (managed by Steiger)

### Bazel

Integrates with [Bazel](https://bazel.build/) builds that output OCI image layouts. Works best with [`rules_oci`](https://github.com/bazel-contrib/rules_oci) for creating OCI-compatible container images.

Key difference from Skaffold: Steiger works directly with OCI image layouts, skipping the TAR export step that Skaffold requires. This allows direct pushing to registries without intermediate file formats.

### Ko

Supports [Ko](https://ko.build/) for building Go applications into container images without Dockerfiles.

### Nix

Integrates with [Nix](https://nixos.org/) flake outputs that produce OCI images.

Requirements

- Flakes enabled (`--extra-experimental-features 'nix-command flakes'`)
- `pkgs.ociTools.buildImage` (available via Steiger overlay or [nixpkgs#390624](https://github.com/NixOS/nixpkgs/pull/390624))

<details>
<summary>Example flake</summary>

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    steiger.url = "github:brainhivenl/steiger";
  };

  outputs = {
    nixpkgs,
    steiger,
    ...
  }: let
    system = "x86_64-linux";
    overlays = [steiger.overlays.ociTools];
    pkgs = import nixpkgs { inherit system overlays; };
  in {
    packages.${system} = {
      default = pkgs.ociTools.buildImage {
        name = "hello";

        copyToRoot = pkgs.buildEnv {
          name = "hello-env";
          paths = [pkgs.hello];
          pathsToLink = ["/bin"];
        };

        config.Cmd = ["/bin/hello"];
        compressor = "none";
      };
    };

    devShells.${system} = {
      default = pkgs.mkShell {
        packages = [steiger.packages.${system}.default];
      };
    };
  };
}
```

</details>

## Build Caching

Steiger delegates caching to the underlying build systems rather than implementing its own cache layer:

- **Docker BuildKit**: Leverages BuildKit's native layer caching and build cache
- **Bazel**: Uses Bazel's extensive caching system (action cache, remote cache, etc.)
- **Ko**: Benefits from Go's build cache and Ko's layer caching
- **Nix**: Utilizes Nix's content-addressed store and binary cache system for reproducible, cached builds

This approach avoids cache invalidation issues and performs comparably to Skaffold in cached scenarios, with better performance in some cases.

## Installation

### Using cargo

```bash
cargo install steiger --git https://github.com/brainhivenl/steiger.git
```

### Using nix

Run directly without installation:

```bash
nix run github:brainhivenl/steiger -- build
```

### Using GitHub Actions

Use the official GitHub Action in your workflows:

```yaml
name: Build and Deploy
on:
  push:
    branches: [main]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: brainhivenl/steiger-action@v1
        with:
          cmd: build
          args: --repo ghcr.io/my-org/my-project
          version: v0.0.1
```

The action supports these inputs:

- `cmd` (required): The steiger command to run (default: `build`)
- `args` (optional): Arguments to pass to the command
- `version` (optional): Version of steiger to use (default: `v0.0.1`)

### Build from source

```bash
git clone https://github.com/brainhivenl/steiger
cd steiger
cargo build --release
```

## Configuration

Create a `steiger.yml` file:

```yaml
build:
  frontend:
    type: docker
    context: ./frontend
    dockerfile: Dockerfile.prod # optional, defaults to Dockerfile
    buildArgs:
      ENV: ${env} # variable substitution is supported

  backend:
    type: bazel
    targets:
      app: //cmd/server:image
      migrations: //cmd/migrations:image

  go-service:
    type: ko
    importPath: ./cmd/service

  flake:
    type: nix
    packages:
      api: default # attribute path to package e.g. `outputs.packages.<system>.default`

deploy:
  brainpod:
    type: helm
    path: helm
    namespace: my-app
    valuesFiles:
      - helm/values.yaml

insecureRegistries:
  - my-registry.localhost:5000

profiles:
  prod:
    env: prod
```

### Bazel Configuration

For Bazel builds, ensure your targets produce OCI image layouts:

```python
# BUILD.bazel
load("@rules_oci//oci:defs.bzl", "oci_image", "oci_tarball")

oci_image(
    name = "image",
    base = "@distroless_base",
    entrypoint = ["/app"],
    tars = [":app_layer"],
)
```

Platform-specific builds:

```yaml
build:
  multi-arch:
    type: bazel
    platforms:
      linux/amd64: //platforms:linux_amd64
      linux/arm64: //platforms:linux_arm64
    targets:
      app: //cmd/app:image
```

## Usage

### Build All Services

```bash
steiger build
```

### Build and Push

```bash
steiger build --repo gcr.io/my-project
```

This will:

1. Build all services in parallel
2. Push to `gcr.io/my-project/{service-name}:latest`
3. Skip redundant pushes based on image digests

### Deploy

Deploy services to Kubernetes based on the `output-file` from the build command:

```bash
steiger deploy
```

The deploy command uses the build metadata to deploy the correct image versions to your Kubernetes cluster.

### Run Full Pipeline

Run the complete pipeline (build, push, and deploy):

```bash
steiger run --repo gcr.io/my-project
```

This command combines all steps:

1. Builds all services in parallel
2. Pushes images to the specified repository
3. Deploys services using the deployment configuration

### Generate Build Metadata

Compatible with Skaffold's build output format:

```bash
steiger build --repo gcr.io/my-project --output-file builds.json
```

Output:

```json
{
  "builds": [
    {
      "imageName": "frontend",
      "tag": "gcr.io/my-project/frontend:latest"
    },
    {
      "imageName": "backend-app",
      "tag": "gcr.io/my-project/backend-app@sha256:abc123..."
    }
  ]
}
```

### Options

```bash
# Custom config location
steiger --config ./deploy/steiger.yml build

# Change working directory
steiger --dir ./monorepo build

# Build and push
steiger build --repo ghcr.io/foo/bar --platform linux/amd64
```

## Platform Detection

Steiger automatically detects the target platform:

1. From Kubernetes cluster context (if available)
2. Host platform detection as fallback

Supported platforms: `linux/amd64`, `linux/arm64`, `darwin/amd64`, `darwin/arm64`, `windows/amd64`

## Registry Authentication

Uses Docker's credential helper system:

```bash
# Docker Hub
docker login

# Google Container Registry
gcloud auth configure-docker

# AWS ECR
aws ecr get-login-password --region us-west-2 | docker login --username AWS --password-stdin $ECR_REGISTRY
```

For insecure HTTP registries (development environments), configure them in your `steiger.yml`:

```yaml
insecureRegistries:
  - localhost:5000
  - dev-registry.local:8080
```

## Architecture

- **Async Runtime**: Built on Tokio for concurrent operations
- **OCI Native**: Direct manipulation of OCI image formats
- **Builder Abstraction**: Extensible system for supporting new build tools
- **Registry Client**: Direct OCI registry operations without Docker daemon

## Comparison with Skaffold

| Aspect            | Steiger                        | Skaffold                    |
| ----------------- | ------------------------------ | --------------------------- |
| Bazel Integration | OCI layout direct from Bazel   | TAR export required         |
| Build Caching     | Delegates to build systems     | Custom cache management     |
| Configuration     | Minimal YAML                   | Comprehensive configuration |
| Deployment        | Planned (use Skaffold for now) | Full lifecycle management   |

## Contributing

This project is under active development, contributions are welcome.
