# Steiger

A modern, high-performance container build orchestrator with native support for Bazel and Docker BuildKit. Steiger streamlines multi-service container builds and registry operations with automatic platform detection and parallel execution. Similar to Skaffold, but optimized for Bazel workflows and OCI images.

## Features

### 🚀 Multi-Builder Support

- **Native Bazel Integration**: First-class support for Bazel builds with automatic target discovery and platform configuration
- **Docker BuildKit**: Leverages Docker's next-generation BuildKit for efficient, cached builds
- **Parallel Execution**: Build multiple services concurrently with real-time progress tracking

### 🎯 Platform Detection

- Automatically detects the target platform from Kubernetes clusters when available
- Falls back to host platform detection (linux/amd64, linux/arm64, darwin/amd64, darwin/arm64, windows/amd64)
- Per-service platform configuration for Bazel builds

### 📦 OCI Registry Integration

- Push built images directly to any OCI-compliant registry
- Digest checking to avoid redundant pushes
- Automatic authentication via Docker credential helpers
- Support for multi-architecture image manifests

### 🔧 Developer Experience

- Real-time build progress with interactive terminal UI
- Skaffold-compatible output format for seamless integration
- Configurable via simple YAML configuration
- Automatic BuildKit builder management

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

Create a `steiger.yml` file in your project root:

```yaml
services:
  frontend:
    build:
      type: docker
      context: ./frontend
      dockerfile: Dockerfile.prod # optional, defaults to ./frontend/Dockerfile

  backend:
    build:
      type: bazel
      targets:
        app: //cmd/server:image
        migrations: //cmd/migrations:image
      platforms:
        linux/amd64: //platforms:linux_amd64
        linux/arm64: //platforms:linux_arm64

  worker:
    build:
      type: docker
      context: ./services/worker
```

## Usage

### Basic Build

Build all services defined in `steiger.yml`:

```bash
steiger build
```

### Build and Push to Registry

```bash
steiger build --repo gcr.io/my-project
```

This will:

1. Build all configured services in parallel
2. Push images to `gcr.io/my-project/{service-name}:latest`
3. Skip pushing if the image already exists (based on digest)

### Generate Skaffold-Compatible Output

For integration with existing Skaffold workflows:

```bash
steiger build --repo gcr.io/my-project --output-file build.json
```

Output format:

```json
{
  "builds": [
    {
      "imageName": "frontend",
      "tag": "gcr.io/my-project/frontend:latest"
    },
    {
      "imageName": "backend-app",
      "tag": "gcr.io/my-project/backend-app@sha256:..."
    }
  ]
}
```

### Custom Configuration Path

```bash
steiger --config ./deploy/steiger.yml build --repo myregistry.io/project
```

### Working Directory

```bash
steiger --dir ./monorepo build
```

## Builder Details

### Bazel Builder

The Bazel builder supports:

- Multiple artifacts per service (e.g., separate app and migration images)
- Platform-specific build configurations
- Automatic output discovery via `bazel cquery`
- Support for both `bazel` and `bazelisk` binaries

Example Bazel rules:

```python
oci_image(
    name = "image",
    base = "@distroless_cc",
    entrypoint = ["/app"],
    tars = [":app_layer"],
)
```

### Docker Builder

The Docker builder features:

- Automatic BuildKit builder creation and management
- Multi-platform builds
- Build context and Dockerfile customization
- OCI format output (not Docker format)
- Efficient layer caching

## Advanced Features

### Platform Configuration

Steiger automatically detects the target platform in the following order:

1. Kubernetes cluster platform (via kubeconfig)
2. Host platform detection

For Bazel builds, you can specify platform mappings:

```yaml
services:
  cross-platform-service:
    build:
      type: bazel
      platforms:
        linux/amd64: //build/platforms:linux_amd64
        linux/arm64: //build/platforms:linux_arm64
        darwin/amd64: //build/platforms:darwin_amd64
      targets:
        app: //cmd/app:image
```

### Registry Authentication

Steiger uses Docker's credential helper system for registry authentication:

```bash
# Login to Docker Hub
docker login

# Login to GCR
docker-credential-gcr configure-docker

# Login to ECR
aws ecr get-login-password | docker login --username AWS --password-stdin $ECR_REGISTRY
```

### Progress Monitoring

Steiger provides detailed progress information during builds:

- Real-time output from build commands
- Progress bars for parallel builds
- Status messages for each build stage
- Automatic terminal UI adaptation

## Architecture

Steiger is built with:

- **Async/Await**: Tokio-based asynchronous runtime for maximum performance
- **Parallel Execution**: Concurrent builds across multiple services
- **Modular Builders**: Extensible builder trait system
- **OCI Native**: Direct OCI format manipulation without Docker daemon dependency

## Comparison with Skaffold

| Feature           | Steiger                                                         | Skaffold                                       |
| ----------------- | --------------------------------------------------------------- | ---------------------------------------------- |
| Bazel Support     | Direct OCI format via `bazel cquery`, leverages Bazel's caching | Requires TAR output, slower metadata gathering |
| Docker BuildKit   | Native, automatic builder management                            | Supported                                      |
| Build Parallelism | Default                                                         | Configurable                                   |
| Progress UI       | Built-in interactive                                            | Text output                                    |
| Configuration     | Simple YAML                                                     | Extensive YAML                                 |

## Acknowledgments

Inspired by [Skaffold](https://skaffold.dev/) with a focus on modern build systems and native Bazel support.
