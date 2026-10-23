import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def body(source, start, end):
    begin = source.index(start)
    return source[begin : source.index(end, begin + len(start))]


class CheckOnStartupGlobalOptionTest(unittest.TestCase):
    """enable-check-update is stored in the machine-wide Config, not LocalConfig."""

    def setUp(self):
        self.ui = (ROOT / "src/ui_interface.rs").read_text(encoding="utf-8")
        self.updater = (ROOT / "src/updater.rs").read_text(encoding="utf-8")
        self.common = (ROOT / "src/common.rs").read_text(encoding="utf-8")

    def test_ui_local_option_entry_points_route_the_key_to_the_global_config(self):
        get = body(self.ui, "pub fn get_local_option(key: String)", "\n}\n")
        set_ = body(self.ui, "pub fn set_local_option(key: String, value: String)", "\n}\n")
        for fn in (get, set_):
            self.assertIn("base::update::is_global_check_on_startup_option(&key)", fn)
        self.assertIn("check_on_startup_option()", get)
        self.assertIn("set_check_on_startup_option(value)", set_)

        read = body(self.ui, "fn check_on_startup_option()", "\n}\n")
        self.assertIn("base::update::resolve_check_on_startup_option(", read)
        self.assertIn("get_option(keys::OPTION_ENABLE_CHECK_UPDATE)", read)
        self.assertIn("LocalConfig::get_option(keys::OPTION_ENABLE_CHECK_UPDATE)", read)
        self.assertIn("option_synced()", read)

        write = body(self.ui, "fn set_check_on_startup_option(value: String)", "\n}\n")
        self.assertIn("base::update::store_check_on_startup_option(", write)
        for fn in (read, write):
            self.assertIn("set_global_check_on_startup_option,", fn)
            self.assertIn("clear_legacy_check_on_startup_option,", fn)

        # set_option goes through the shared option map and IPC to the service.
        global_write = body(self.ui, "fn set_global_check_on_startup_option(value: &str)", "\n}\n")
        self.assertIn("    set_option(\n        keys::OPTION_ENABLE_CHECK_UPDATE.to_owned(),", global_write)
        legacy_clear = body(self.ui, "fn clear_legacy_check_on_startup_option()", "\n}\n")
        self.assertIn("LocalConfig::set_option(keys::OPTION_ENABLE_CHECK_UPDATE.to_owned(), String::new())", legacy_clear)

    def test_update_policy_writes_the_global_config(self):
        policy = body(self.updater, "fn apply_update_policy(", "\nfn ")
        self.assertNotIn("LocalConfig", policy)
        self.assertIn(
            "config::Config::set_option(\n        keys::OPTION_ENABLE_CHECK_UPDATE.to_owned(),",
            policy,
        )

    def test_service_reads_the_global_value_with_the_legacy_copy_as_fallback(self):
        startup = body(self.updater, "fn startup_update_enabled()", "\n}\n")
        self.assertIn("base::update::check_on_startup_enabled(", startup)
        self.assertIn("config::Config::get_option(keys::OPTION_ENABLE_CHECK_UPDATE)", startup)

        sysinfo = body(self.common, 'out["enable_check_update"]', ";\n")
        self.assertIn("base::update::check_on_startup_enabled(", sysinfo)
        self.assertIn("Config::get_option(keys::OPTION_ENABLE_CHECK_UPDATE)", sysinfo)

        legacy_ui = body(self.common, "pub fn check_software_update()", "\n}\n")
        self.assertNotIn("LocalConfig::get_option(keys::OPTION_ENABLE_CHECK_UPDATE)", legacy_ui)
        self.assertIn("crate::ui_interface::get_local_option(", legacy_ui)


if __name__ == "__main__":
    unittest.main()
