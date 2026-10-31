import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
RUNNER = REPO_ROOT / "flutter/macos/Runner"


class MacosDockReopenTest(unittest.TestCase):
    def test_app_delegate_forwards_dock_reopen_to_the_main_engine(self) -> None:
        source = (RUNNER / "AppDelegate.swift").read_text(encoding="utf-8")
        start = source.index("override func applicationShouldHandleReopen(")
        body = source[start : source.index("\n    }\n", start)]
        self.assertIn("hasVisibleWindows flag: Bool", body)
        self.assertIn("if !flag", body)
        self.assertIn('MainFlutterWindow.mainHostChannel?.invokeMethod("reopenMainWindow", arguments: nil)', body)
        self.assertIn("return true", body)
        self.assertIn(
            "return false",
            source[source.index("applicationShouldTerminateAfterLastWindowClosed") :],
        )

    def test_main_window_keeps_its_own_host_channel(self) -> None:
        source = (RUNNER / "MainFlutterWindow.swift").read_text(encoding="utf-8")
        self.assertIn("static var mainHostChannel: FlutterMethodChannel?", source)
        awake = source[source.index("override func awakeFromNib()") :]
        awake = awake[: awake.index("super.awakeFromNib()")]
        self.assertIn("MainFlutterWindow.mainHostChannel = setMethodHandler(registrar: registrar)", awake)

    def test_dart_registers_the_reopen_handler_for_the_main_window(self) -> None:
        main = (REPO_ROOT / "flutter/lib/main.dart").read_text(encoding="utf-8")
        self.assertIn(
            "RdPlatformChannel.instance.setReopenMainWindowHandler(() => windowOnTop(null));",
            main,
        )

    def test_last_window_close_consults_the_policy_before_terminating(self) -> None:
        common = (REPO_ROOT / "flutter/lib/common.dart").read_text(encoding="utf-8")
        handler = common[common.index("Future<void> onActiveWindowChanged() async {") :]
        handler = handler[: handler.index("\nTimer periodic_immediate(")]
        policy = handler.index("if (!quitWhenAllWindowsClosed(isMacOS: isMacOS)) {")
        self.assertLess(policy, handler.index("await windowManager.close();"))
        self.assertLess(policy, handler.index("RdPlatformChannel.instance.terminate"))

    def test_quit_goes_offline_natively_for_every_engine(self) -> None:
        delegate = (RUNNER / "AppDelegate.swift").read_text(encoding="utf-8")
        self.assertIn('@_silgen_name("rustdesk_mac_will_quit")', delegate)
        start = delegate.index("override func applicationWillTerminate(_ notification: Notification)")
        body = delegate[start : delegate.index("\n    }\n", start)]
        self.assertIn("rustdeskMacWillQuit()", body)
        flutter_rs = (REPO_ROOT / "src/flutter.rs").read_text(encoding="utf-8")
        export = flutter_rs[flutter_rs.index('pub extern "C" fn rustdesk_mac_will_quit()') :]
        export = export[: export.index("\n}\n")]
        self.assertIn("macos_tcc::go_offline_for_quit()", export)
        guard = export.index("if !crate::is_main() {")
        self.assertLess(guard, export.index("go_offline_for_quit()"),
                        "the connection manager process must never take the device offline")

    def test_logout_restart_and_shutdown_are_not_user_quits(self) -> None:
        delegate = (RUNNER / "AppDelegate.swift").read_text(encoding="utf-8")
        self.assertIn('@_silgen_name("rustdesk_mac_mark_programmatic_quit")', delegate)
        self.assertIn("NSWorkspace.willPowerOffNotification", delegate)
        launch = delegate[delegate.index("override func applicationDidFinishLaunching(") :]
        launch = launch[: launch.index("\n    }\n")]
        self.assertIn("NSWorkspace.shared.notificationCenter.addObserver", launch)
        self.assertIn("rustdeskMacMarkProgrammaticQuit()", launch)
        flutter_rs = (REPO_ROOT / "src/flutter.rs").read_text(encoding="utf-8")
        export = flutter_rs[flutter_rs.index('pub extern "C" fn rustdesk_mac_mark_programmatic_quit()') :]
        export = export[: export.index("\n}\n")]
        self.assertIn("macos_tcc::mark_programmatic_quit()", export)

    def test_programmatic_exits_are_marked_before_terminating(self) -> None:
        ipc = (REPO_ROOT / "src/ipc.rs").read_text(encoding="utf-8")
        close = ipc[ipc.index("Data::Close => {") :]
        close = close[: close.index("std::process::exit(-1);")]
        mark = close.index("crate::platform::macos_tcc::mark_programmatic_quit();")
        self.assertLess(mark, close.index("crate::platform::quit_gui();"))
        runtime = (
            REPO_ROOT / "flutter/lib/desktop/widgets/mac_permission_guide_runtime.dart"
        ).read_text(encoding="utf-8")
        relaunch = runtime[runtime.index("Future<void> _relaunch() async {") :]
        self.assertLess(
            relaunch.index("await Process.start('/bin/sh'"),
            relaunch.index("key: 'mac-programmatic-quit'"),
            "only mark after the relaunch helper really started",
        )
        self.assertLess(
            relaunch.index("key: 'mac-programmatic-quit'"),
            relaunch.index("RdPlatformChannel.instance.terminate"),
        )

    def test_main_window_restores_online_state_and_ffi_keys_exist(self) -> None:
        home = (REPO_ROOT / "flutter/lib/desktop/pages/desktop_home_page.dart").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            "installMacQuitOffline((k) => bind.mainSetCommon(key: k, value: 'Y'));", home
        )
        ffi = (REPO_ROOT / "src/flutter_ffi.rs").read_text(encoding="utf-8")
        setter = ffi[ffi.index("pub fn main_set_common(") :]
        setter = setter[: setter.index("\n}\n")]
        self.assertIn('_key == "mac-programmatic-quit"', setter)
        self.assertIn('_key == "mac-restore-after-quit"', setter)
        self.assertNotIn('"mac-quit-offline"', ffi)
        getter = ffi[ffi.index("pub fn main_get_common(") :]
        getter = getter[: getter.index("\n}\n")]
        self.assertIn('key.strip_prefix("mac-admin-setup:")', getter)

if __name__ == "__main__":
    unittest.main()
