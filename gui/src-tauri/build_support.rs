//! Target selection shared by sidecar staging and its standalone regression
//! tests. These checks need no webview or downloaded AllMyStuff dependency:
//! `rustc --test gui/src-tauri/build_support.rs -o target/build-support-tests`.

#[derive(Clone, Copy)]
pub enum AssetStyle {
    Standard,
    CrucibleWindows,
}

impl AssetStyle {
    pub fn supports_target(self, triple: &str) -> bool {
        match self {
            Self::Standard => true,
            Self::CrucibleWindows => triple.contains("windows") && triple.contains("x86_64"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AssetStyle;

    #[test]
    fn linux_and_macos_release_sidecars_exclude_crucible() {
        for triple in [
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
        ] {
            assert!(AssetStyle::Standard.supports_target(triple));
            assert!(!AssetStyle::CrucibleWindows.supports_target(triple));
        }
    }

    #[test]
    fn windows_x64_keeps_the_required_crucible_payload() {
        let triple = "x86_64-pc-windows-msvc";
        assert!(AssetStyle::Standard.supports_target(triple));
        assert!(AssetStyle::CrucibleWindows.supports_target(triple));
        assert!(!AssetStyle::CrucibleWindows.supports_target("aarch64-pc-windows-msvc"));
    }
}
