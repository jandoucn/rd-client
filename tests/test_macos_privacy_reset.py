import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]


class MacosPrivacyResetHookTest(unittest.TestCase):
    def setUp(self) -> None:
        source = (REPO_ROOT / "src/platform/macos.rs").read_text(encoding="utf-8")
        start = source.index("pub fn update_from_dmg_as_root(")
        self.function = source[start : source.index("\n}\n", start)]

    def test_reset_functions_are_defined_before_the_transaction_starts(self) -> None:
        script = self.function
        definitions = script.index('privacy_uids="{uid_list}"\n{stale_privacy_reset}\n')
        self.assertLess(definitions, script.index("trap rollback_transaction EXIT\n"))
        self.assertEqual(script.count("{stale_privacy_reset}"), 1)

    def test_managed_reset_runs_only_after_commit_and_before_gui_relaunch(self) -> None:
        script = self.function
        commit = script.index("trap - HUP INT TERM\n")
        managed = script.index(
            'if [ "$install_topology" = "managed" ]; then\n    reset_stale_privacy\n    relaunch_gui ||',
            commit,
        )
        self.assertLess(commit, managed)
        self.assertLess(script.index("rollback_done=1\nbundle_swapped=0\n"), managed)

    def test_standalone_reset_runs_after_verification_right_before_gui_relaunch(self) -> None:
        script = self.function
        verify = script.index('elif ! codesign --verify --deep --strict "{app_bundle}"')
        relaunch = script.index(
            'elif [ -z "$gui_uids" ] || ! reset_stale_privacy || ! relaunch_gui; then', verify
        )
        self.assertLess(verify, relaunch)

    def test_reset_is_called_exactly_twice_and_never_before_the_swap(self) -> None:
        script = self.function
        calls = [i for i in range(len(script)) if script.startswith("reset_stale_privacy", i)]
        swap = script.index('if ! mv "$staged_bundle" "{app_bundle}"; then')
        invocations = [i for i in calls if not script.startswith("reset_stale_privacy()", i)]
        self.assertEqual(len(invocations), 2)
        self.assertTrue(all(i > swap for i in invocations))

    def test_rollback_after_reset_is_reported_as_critical(self) -> None:
        script = self.function
        rollback = script[script.index("rollback_transaction() {{") :]
        rollback = rollback[: rollback.index("\n}}\n")]
        self.assertIn('if [ "$privacy_reset_done" -ne 0 ]; then', rollback)
        self.assertIn("CRITICAL: rolled back after stale privacy grants were reset", rollback)
        self.assertIn("/usr/bin/logger -t rustdesk-root-update", rollback)

    def test_signing_identity_is_read_before_the_final_race_window(self) -> None:
        script = self.function
        call = script.index("super::macos_tcc::stale_privacy_reset_for_update(")
        self.assertLess(call, script.index("std::thread::sleep(std::time::Duration::from_secs(3));"))
        args = script[call : script.index(");", call)]
        for expected in (
            "&app_bundle",
            "&src_app",
            '"privacy_uids"',
            "rustdesk_root_update.log",
            "privacy_handled_marker(&runtime_app_name, event.build_seq)",
        ):
            self.assertIn(expected, args)
        self.assertIn("stale_privacy_reset = stale_privacy_reset,", script)


if __name__ == "__main__":
    unittest.main()
