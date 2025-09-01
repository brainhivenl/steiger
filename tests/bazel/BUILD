load("@rules_oci//oci:defs.bzl", "oci_image")
load("@rules_pkg//pkg:tar.bzl", "pkg_tar")

pkg_tar(
    name = "hello_tar",
    srcs = ["hello.txt"],
    package_dir = "/data",
)

oci_image(
    name = "hello_image",
    base = "@alpine",
    entrypoint = ["/bin/cat", "/data/hello.txt"],
    tars = [":hello_tar"],
)

platform(
    name = "linux-arm64",
    constraint_values = [
        "@platforms//os:linux",
        "@platforms//cpu:arm64",
    ],
)

platform(
    name = "linux-amd64",
    constraint_values = [
        "@platforms//os:linux",
        "@platforms//cpu:x86_64",
    ],
)

platform(
    name = "macos-arm64",
    constraint_values = [
        "@platforms//os:macos",
        "@platforms//cpu:arm64",
    ],
)
