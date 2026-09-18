use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const PRODUCT: &str = "Gooey Kick POC";
const BUNDLE_ID: &str = "audio.gooey.kick-poc";
const PLUGINVAL_URL: &str =
    "https://github.com/Tracktion/pluginval/releases/download/v1.0.4/pluginval_macOS.zip";
const PLUGINVAL_SHA256: &str = "3c4c533bda0c5059eea3ddaea752d757ee2025041f0f47e6bcb0e87f6082b29f";

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    ensure_macos()?;
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("build-vst3" | "bundle-vst3") => {
            reject_extra_args(args)?;
            let bundle = bundle_vst3()?;
            println!("Built {}", bundle.display());
        }
        Some("install-vst3") => {
            let force = parse_force(args)?;
            let bundle = bundle_vst3()?;
            let destination = install_vst3(&bundle, force)?;
            println!("Installed {}", destination.display());
        }
        Some("validate-vst3") => {
            reject_extra_args(args)?;
            let bundle = bundle_vst3()?;
            validate_vst3(&bundle)?;
        }
        _ => print_help(),
    }
    Ok(())
}

fn print_help() {
    eprintln!(
        "Usage: cargo run -p gooey-kick-xtask -- <command>\n\n\
         Commands:\n  bundle-vst3         Build, assemble, inspect, and ad-hoc sign the VST3\n  \
         install-vst3 [--force]  Install to the current user's VST3 directory\n  \
         validate-vst3       Bundle and run pluginval 1.0.4 at strictness 5"
    );
}

fn ensure_macos() -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err("Gooey Kick POC packaging is supported on macOS only".into())
    }
}

fn reject_extra_args(mut args: impl Iterator<Item = String>) -> Result<()> {
    if let Some(argument) = args.next() {
        Err(format!("unexpected argument: {argument}").into())
    } else {
        Ok(())
    }
}

fn parse_force(args: impl Iterator<Item = String>) -> Result<bool> {
    let arguments: Vec<_> = args.collect();
    match arguments.as_slice() {
        [] => Ok(false),
        [flag] if flag == "--force" => Ok(true),
        _ => Err("install-vst3 accepts only the optional --force flag".into()),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is directly under the workspace root")
        .to_path_buf()
}

fn bundle_path() -> PathBuf {
    workspace_root()
        .join("target/bundled")
        .join(format!("{PRODUCT}.vst3"))
}

fn executable_path(bundle: &Path) -> PathBuf {
    bundle.join("Contents/MacOS").join(PRODUCT)
}

fn bundle_vst3() -> Result<PathBuf> {
    let root = workspace_root();
    run_command(
        Command::new("cargo").current_dir(&root).args([
            "build",
            "--release",
            "-p",
            "gooey-kick-vst3",
            "--lib",
        ]),
        "building the release VST3 library",
    )?;

    let source = root.join("target/release/libgooey_kick_vst3.dylib");
    if !source.is_file() {
        return Err(format!("release library was not produced at {}", source.display()).into());
    }
    let bundle = bundle_path();
    if bundle.exists() {
        fs::remove_dir_all(&bundle)?;
    }
    let contents = bundle.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    fs::create_dir_all(&macos)?;
    fs::create_dir_all(&resources)?;
    fs::copy(&source, executable_path(&bundle))?;
    fs::write(contents.join("Info.plist"), info_plist())?;
    fs::write(resources.join("moduleinfo.json"), module_info())?;

    run_command(
        Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(contents.join("Info.plist")),
        "validating Info.plist",
    )?;
    let executable = executable_path(&bundle);
    let archs = command_output(
        Command::new("/usr/bin/lipo").arg("-archs").arg(&executable),
        "inspecting executable architecture",
    )?;
    if archs.trim() != "arm64" {
        return Err(format!("expected an arm64-only executable, found: {}", archs.trim()).into());
    }
    let symbols = command_output(
        Command::new("/usr/bin/nm").args([OsStr::new("-gU"), executable.as_os_str()]),
        "inspecting exported entry symbols",
    )?;
    for symbol in ["_GetPluginFactory", "_bundleEntry", "_bundleExit"] {
        if !symbols.lines().any(|line| line.ends_with(symbol)) {
            return Err(format!("required entry symbol {symbol} is missing").into());
        }
    }
    run_command(
        Command::new("/usr/bin/codesign")
            .args(["--force", "--deep", "--sign", "-"])
            .arg(&bundle),
        "ad-hoc signing the VST3 bundle",
    )?;
    run_command(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict", "--verbose=2"])
            .arg(&bundle),
        "verifying the VST3 signature",
    )?;
    Ok(bundle)
}

fn info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>{PRODUCT}</string>
  <key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>{PRODUCT}</string>
  <key>CFBundlePackageType</key><string>BNDL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
</dict>
</plist>
"#
    )
}

fn module_info() -> &'static str {
    r#"{
  "Name": "Gooey Kick POC",
  "Version": "0.1.0",
  "Factory Info": {
    "Vendor": "Gooey Audio",
    "URL": "https://github.com/gooey-audio/libgooey",
    "E-Mail": ""
  },
  "Classes": [
    {
      "CID": "7D29F21835C84BDFA7E7D903B4896721",
      "Category": "Audio Module Class",
      "Name": "Gooey Kick POC",
      "Vendor": "Gooey Audio",
      "Version": "0.1.0",
      "SDKVersion": "VST 3.7",
      "Sub Categories": ["Instrument", "Drum"]
    }
  ]
}
"#
}

fn install_vst3(bundle: &Path, force: bool) -> Result<PathBuf> {
    let home =
        env::var_os("HOME").ok_or("HOME is not set; cannot locate the user VST3 directory")?;
    let directory = PathBuf::from(home).join("Library/Audio/Plug-Ins/VST3");
    let destination = directory.join(format!("{PRODUCT}.vst3"));
    fs::create_dir_all(&directory)?;

    if destination.exists() {
        let existing_id = bundle_identifier(&destination).ok();
        if existing_id.as_deref() != Some(BUNDLE_ID) && !force {
            return Err(format!(
                "refusing to overwrite unrelated bundle at {}; rerun with --force only after verifying it",
                destination.display()
            )
            .into());
        }
    }

    let staging = directory.join(format!(".{PRODUCT}.vst3.installing"));
    let backup = directory.join(format!(".{PRODUCT}.vst3.backup"));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    if backup.exists() {
        fs::remove_dir_all(&backup)?;
    }
    copy_directory(bundle, &staging)?;
    if destination.exists() {
        fs::rename(&destination, &backup)?;
    }
    if let Err(error) = fs::rename(&staging, &destination) {
        if backup.exists() {
            let _ = fs::rename(&backup, &destination);
        }
        return Err(error.into());
    }
    if backup.exists() {
        fs::remove_dir_all(backup)?;
    }
    Ok(destination)
}

fn bundle_identifier(bundle: &Path) -> Result<String> {
    command_output(
        Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleIdentifier", "raw", "-o", "-"])
            .arg(bundle.join("Contents/Info.plist")),
        "reading the installed bundle identifier",
    )
    .map(|identifier| identifier.trim().to_owned())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn validate_vst3(bundle: &Path) -> Result<()> {
    let pluginval = ensure_pluginval()?;
    println!(
        "Running pluginval 1.0.4 strictness 5 with GUI tests: {}",
        bundle.display()
    );
    run_command(
        Command::new(pluginval)
            .arg("--validate")
            .arg(bundle)
            .args(["--strictness-level", "5"]),
        "validating Gooey Kick POC",
    )
}

fn ensure_pluginval() -> Result<PathBuf> {
    let tools = workspace_root().join("target/tools");
    let extracted = tools.join("pluginval-1.0.4");
    let executable = extracted.join("pluginval.app/Contents/MacOS/pluginval");
    if executable.is_file() {
        return Ok(executable);
    }
    fs::create_dir_all(&tools)?;
    let archive = tools.join("pluginval_macOS-1.0.4.zip");
    if !archive.is_file() {
        let temporary = tools.join("pluginval_macOS-1.0.4.zip.download");
        if temporary.exists() {
            fs::remove_file(&temporary)?;
        }
        run_command(
            Command::new("/usr/bin/curl")
                .args(["--fail", "--location", "--output"])
                .arg(&temporary)
                .arg(PLUGINVAL_URL),
            "downloading pluginval 1.0.4",
        )?;
        fs::rename(temporary, &archive)?;
    }
    let checksum = command_output(
        Command::new("/usr/bin/shasum")
            .args(["-a", "256"])
            .arg(&archive),
        "checking pluginval SHA-256",
    )?;
    let actual = checksum.split_whitespace().next().unwrap_or_default();
    if actual != PLUGINVAL_SHA256 {
        fs::remove_file(&archive)?;
        return Err(format!(
            "pluginval checksum mismatch: expected {PLUGINVAL_SHA256}, got {actual}; removed the cached archive"
        )
        .into());
    }
    println!("Verified pluginval SHA-256 {PLUGINVAL_SHA256}");
    if extracted.exists() {
        fs::remove_dir_all(&extracted)?;
    }
    fs::create_dir_all(&extracted)?;
    run_command(
        Command::new("/usr/bin/unzip")
            .arg("-oq")
            .arg(&archive)
            .arg("-d")
            .arg(&extracted),
        "extracting pluginval",
    )?;
    if !executable.is_file() {
        return Err(format!("pluginval executable not found at {}", executable.display()).into());
    }
    Ok(executable)
}

fn run_command(command: &mut Command, context: &str) -> Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{context} failed with {status}").into())
    }
}

fn command_output(command: &mut Command, context: &str) -> Result<String> {
    let Output {
        status,
        stdout,
        stderr,
    } = command.output()?;
    if !status.success() {
        return Err(format!(
            "{context} failed with {status}: {}",
            String::from_utf8_lossy(&stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8(stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_contains_required_product_metadata() {
        let plist = info_plist();
        assert!(plist.contains(BUNDLE_ID));
        assert!(plist.contains(PRODUCT));
        assert!(plist.contains("<string>0.1.0</string>"));
    }

    #[test]
    fn module_info_contains_stable_processor_id_and_category() {
        let info = module_info();
        assert!(info.contains("7D29F21835C84BDFA7E7D903B4896721"));
        assert!(info.contains("\"Instrument\", \"Drum\""));
    }
}
