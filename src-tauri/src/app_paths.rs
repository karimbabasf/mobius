use std::path::PathBuf;

fn home() -> PathBuf {
    dirs::home_dir().expect("home directory is required for app data paths")
}

pub fn data_dir() -> PathBuf {
    home().join("Library/Application Support/Mobius")
}

pub fn db_path() -> PathBuf {
    data_dir().join("state.sqlite")
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

pub fn pricing_cache_path() -> PathBuf {
    data_dir().join("pricing-cache.json")
}

pub fn socket_path() -> PathBuf {
    data_dir().join("collector.sock")
}

pub fn ensure_dirs() -> std::io::Result<()> {
    std::fs::create_dir_all(logs_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_is_under_app_support_and_named_correctly() {
        let d = data_dir();
        let s = d.to_string_lossy();
        assert!(s.contains("Library/Application Support/Mobius"), "got {s}");
    }

    #[test]
    fn db_path_is_state_sqlite_inside_data_dir() {
        assert_eq!(db_path().file_name().unwrap(), "state.sqlite");
        assert_eq!(db_path().parent().unwrap(), data_dir());
    }

    #[test]
    fn support_paths_are_all_inside_data_dir() {
        assert_eq!(logs_dir().file_name().unwrap(), "logs");
        assert_eq!(logs_dir().parent().unwrap(), data_dir());
        assert_eq!(
            pricing_cache_path().file_name().unwrap(),
            "pricing-cache.json"
        );
        assert_eq!(pricing_cache_path().parent().unwrap(), data_dir());
        assert_eq!(socket_path().file_name().unwrap(), "collector.sock");
        assert_eq!(socket_path().parent().unwrap(), data_dir());
    }
}
