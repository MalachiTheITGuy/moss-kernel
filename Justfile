#!/usr/bin/env just --justfile
#
# Moss Kernel Justfile
#
# Targets:
#   x86_64  - Default architecture for `just run`
#   aarch64 - ARM64 architecture
#

#
# Recipes
#

# Create the moss.img initrd image for the specified architecture
# Usage: just create-image [arch]
# Examples:
#   just create-image        # creates x86_64 image (default)
#   just create-image x86_64 # creates x86_64 image
#   just create-image aarch64 # creates aarch64 image
create-image arch="x86_64":
    ./scripts/create-image.sh {{ arch }}

# Run the kernel via QEMU for the specified architecture
# Usage: just run [arch]
# Examples:
#   just run         # runs x86_64 kernel via QEMU (default)
#   just run x86_64  # runs x86_64 kernel via QEMU
#   just run aarch64 # runs aarch64 kernel via QEMU
run arch="x86_64":
    #!/usr/bin/env sh
    # Validate architecture
    case "{{ arch }}" in
        x86_64|aarch64) ;;
        *) echo "Error: unsupported architecture '{{ arch }}'" >&2; exit 1 ;;
    esac

    # Create moss.img if it doesn't exist
    if [ ! -f moss.img ]; then
        just create-image {{ arch }}
    fi

    # Determine target based on architecture
    case "{{ arch }}" in
        x86_64)  target="x86_64-unknown-none" ;;
        aarch64) target="aarch64-unknown-none-softfloat" ;;
    esac

    # Build and run via QEMU
    cargo run --release --target "$target" -- /bin/ash

# Run unit tests (same architecture as host)
test-unit:
    #!/usr/bin/env sh
    host_target="$(rustc --version --verbose | awk -F': ' '/^host:/ {print $2; exit}')"
    cargo test --package libkernel --target "$host_target"

# Run KUnit tests
test-kunit:
    cargo test --release

# Run userspace tests
test-userspace:
    cargo run -r -- /bin/usertest
