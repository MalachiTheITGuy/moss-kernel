use std::path::PathBuf;
use time::OffsetDateTime;
use time::macros::format_description;

fn main() {
    let linker_script = match std::env::var("CARGO_CFG_TARGET_ARCH") {
        Ok(arch) if arch == "aarch64" => PathBuf::from("./src/arch/arm64/boot/linker.ld"),
        Ok(arch) if arch == "x86_64" => PathBuf::from("./src/arch/x86_64/boot/linker.ld"),
        Ok(arch) => {
            println!("cargo::warning=Unsupported arch: {arch}");
            std::process::exit(1);
        }
        Err(_) => unreachable!("Cargo should always set the arch"),
    };

    println!("cargo::rerun-if-changed={}", linker_script.display());
    println!("cargo::rustc-link-arg=-T{}", linker_script.display());

    // Tell rust-lld to produce a non-PIE binary on x86_64 so that
    // absolute relocations (R_X86_64_64) in the ISR stub table are accepted.
    if std::env::var("CARGO_CFG_TARGET_ARCH")
        .as_deref()
        .is_ok_and(|a| a == "x86_64")
    {
        println!("cargo::rustc-link-arg=-no-pie");
    }

    // Compile x86_64 assembly files via the cc crate.
    // Uppercase .S files contain #define macros and backslash continuations
    // that the Rust global_asm! macro cannot handle (causes LLVM crashes).
    // Lowercase .s files (idle, vdso) are compiled here to eliminate
    // global_asm! invocations, which also trigger LLVM backend crashes
    // in debug builds.
    if std::env::var("CARGO_CFG_TARGET_ARCH")
        .as_deref()
        .is_ok_and(|a| a == "x86_64")
    {
        cc::Build::new()
            .file("src/arch/x86_64/boot/start.S")
            .flag("-fno-pie")
            .flag("-fno-pic")
            .compile("start");
        cc::Build::new()
            .file("src/arch/x86_64/exceptions/entry.S")
            .flag("-fno-pie")
            .flag("-fno-pic")
            .compile("entry");
        cc::Build::new()
            .file("src/arch/x86_64/proc/idle.s")
            .flag("-fno-pie")
            .flag("-fno-pic")
            .compile("idle");
        cc::Build::new()
            .file("src/arch/x86_64/proc/vdso.s")
            .flag("-fno-pie")
            .flag("-fno-pic")
            .compile("vdso");
    }

    // Set an environment variable with the date and time of the build
    let now = OffsetDateTime::now_utc();
    let format = format_description!(
        "[weekday repr:short] [month repr:short] [day] [hour]:[minute]:[second] UTC [year]"
    );
    let timestamp = now.format(&format).unwrap();
    #[cfg(feature = "smp")]
    println!("cargo:rustc-env=MOSS_VERSION=#1 Moss SMP {timestamp}");
    #[cfg(not(feature = "smp"))]
    println!("cargo:rustc-env=MOSS_VERSION=#1 Moss {timestamp}");
}
