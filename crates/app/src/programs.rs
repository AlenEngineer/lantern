//! The programs Lantern starts: `pdftoppm`, LibreOffice, `yt-dlp`, a
//! shell for the operator's own commands.
//!
//! The Windows download carries poppler in a `poppler` folder beside
//! `lantern.exe`, and a program can be dropped next to it (`yt-dlp.exe`):
//! those are used before any on the `PATH`. And there, a program started
//! from a window app gets no console window of its own.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The folders beside Lantern a program is looked for in, in order.
const BESIDE: [&str; 2] = ["", "poppler"];

/// Windows' `CREATE_NO_WINDOW`: the program runs without a console.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A command that starts `name`: the copy beside Lantern when there is
/// one, else whatever the `PATH` finds.
pub fn command(name: &str) -> Command {
    let here = std::env::current_exe().ok();
    let command = Command::new(find(name, here.as_deref().and_then(Path::parent)));
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt;
        let mut command = command;
        command.creation_flags(CREATE_NO_WINDOW);
        command
    };
    command
}

/// `name` in one of the [`BESIDE`] folders of `dir`, or just `name`.
fn find(name: &str, dir: Option<&Path>) -> PathBuf {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    dir.into_iter()
        .flat_map(|dir| BESIDE.map(|folder| dir.join(folder).join(&file)))
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe(name: &str) -> String {
        format!("{name}{}", std::env::consts::EXE_SUFFIX)
    }

    #[test]
    fn a_program_beside_lantern_or_in_its_poppler_folder_comes_before_the_path() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            find("pdftoppm", Some(dir.path())),
            PathBuf::from("pdftoppm")
        );
        assert_eq!(find("pdftoppm", None), PathBuf::from("pdftoppm"));

        std::fs::create_dir(dir.path().join("poppler")).unwrap();
        let poppler = dir.path().join("poppler").join(exe("pdftoppm"));
        std::fs::write(&poppler, "").unwrap();
        assert_eq!(find("pdftoppm", Some(dir.path())), poppler);

        let beside = dir.path().join(exe("pdftoppm"));
        std::fs::write(&beside, "").unwrap();
        assert_eq!(find("pdftoppm", Some(dir.path())), beside);
        assert_eq!(find("yt-dlp", Some(dir.path())), PathBuf::from("yt-dlp"));
    }

    #[test]
    fn a_full_path_is_left_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let soffice = r"C:\Program Files\LibreOffice\program\soffice.exe";
        assert_eq!(find(soffice, Some(dir.path())), PathBuf::from(soffice));
    }
}
