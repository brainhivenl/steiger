# Steiger

A container build orchestrator for multi-service projects with native support for Bazel, Docker BuildKit, and Ko. Steiger coordinates parallel builds and handles registry operations with automatic platform detection.

## Project Status

- ✅ **Basic building and pushing**: Core functionality is stable
- ✅ **Multi-service parallel builds**: Working with real-time progress
- ✅ **Registry integration**: Push to any OCI-compliant registry
- ⏳ **Dev mode**: File watching and rebuild-on-change (planned)
- ⏳ **Deploy**: Native Kubernetes deployment support (planned)

For now, you can use [Skaffold](https://skaffold.dev/) to deploy images built by Steiger using the compatible JSON output format.

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

## Build Caching

Steiger delegates caching to the underlying build systems rather than implementing its own cache layer:

- **Docker BuildKit**: Leverages BuildKit's native layer caching and build cache
- **Bazel**: Uses Bazel's extensive caching system (action cache, remote cache, etc.)
- **Ko**: Benefits from Go's build cache and Ko's layer caching

This approach avoids cache invalidation issues and performs comparably to Skaffold in cached scenarios, with better performance in some cases.

## Installation

```bash
cargo install steiger
```

Or build from source:

```bash
git clone https://github.com/yourusername/steiger
cd steiger
cargo build --release
```

## Configuration

Create a `steiger.yml` file:

```yaml
services:
  frontend:
    build:
      type: docker
      context: ./frontend
      dockerfile: Dockerfile.prod # optional, defaults to Dockerfile
      buildArgs:
        ENV: ${env} # variable substitution is supported

  backend:
    build:
      type: bazel
      targets:
        app: //cmd/server:image
        migrations: //cmd/migrations:image

  go-service:
    build:
      type: ko
      importPath: ./cmd/service

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
services:
  multi-arch:
    build:
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
