use std::env;
use std::process::Command;

/// Version reported when there is no tag and no git history to read.
const FALLBACK_VERSION: &str = "0.0.0-dev";

/// Resolves the version the binary reports, at build time.
///
/// The release tag is the source of truth: no manifest in this workspace
/// carries the release version, so a release build reads the tag CI is
/// building and a local build describes its distance from the last one.
/// `RUSTMAIL_VERSION` overrides both, for a build with no git history.
fn main() {
  println!("cargo::rerun-if-env-changed=RUSTMAIL_VERSION");
  println!("cargo::rerun-if-env-changed=GITHUB_REF_TYPE");
  println!("cargo::rerun-if-env-changed=GITHUB_REF_NAME");
  println!("cargo::rustc-env=RUSTMAIL_VERSION={}", resolve_version());
}

fn resolve_version() -> String {
  if let Some(explicit) = non_empty_var("RUSTMAIL_VERSION") {
    return strip_tag_prefix(&explicit);
  }
  if non_empty_var("GITHUB_REF_TYPE").as_deref() == Some("tag")
    && let Some(tag) = non_empty_var("GITHUB_REF_NAME")
  {
    return strip_tag_prefix(&tag);
  }
  describe_head().unwrap_or_else(|| FALLBACK_VERSION.to_owned())
}

fn non_empty_var(key: &str) -> Option<String> {
  env::var(key).ok().filter(|value| !value.is_empty())
}

/// Drops the `v` a release tag carries, leaving a commit hash untouched.
fn strip_tag_prefix(version: &str) -> String {
  match version.strip_prefix('v') {
    Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest.to_owned(),
    _ => version.to_owned(),
  }
}

fn describe_head() -> Option<String> {
  let output = Command::new("git")
    .args(["describe", "--tags", "--dirty", "--always"])
    .output()
    .ok()?;
  if !output.status.success() {
    return None;
  }
  let described = String::from_utf8(output.stdout).ok()?;
  let described = described.trim();
  if described.is_empty() {
    return None;
  }
  Some(strip_tag_prefix(described))
}
