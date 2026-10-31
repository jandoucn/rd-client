import Cocoa
import FlutterMacOS

// Exported by the Rust core (src/flutter.rs); declared here so it does not
// depend on the generated bridge header.
@_silgen_name("rustdesk_mac_will_quit")
private func rustdeskMacWillQuit()

@_silgen_name("rustdesk_mac_mark_programmatic_quit")
private func rustdeskMacMarkProgrammaticQuit()

@main
class AppDelegate: FlutterAppDelegate {
    var launched = false;
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
      dummy_method_to_enforce_bundling()
    // https://github.com/leanflutter/window_manager/issues/214
    return false
  }
    
    // Closing the main window only hides it on macOS (the process keeps the
    // device online), so a Dock click must bring that hidden window back.
    override func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag {
            MainFlutterWindow.mainHostChannel?.invokeMethod("reopenMainWindow", arguments: nil)
        }
        return true
    }

    // Cmd+Q and Dock "Quit" take the device offline here rather than in Dart:
    // Flutter sends exit requests to whichever engine registered last. The
    // Flutter delegate does not implement this, so super must not be called.
    override func applicationWillTerminate(_ notification: Notification) {
        rustdeskMacWillQuit()
    }

    override func applicationShouldOpenUntitledFile(_ sender: NSApplication) -> Bool {
        if (launched) {
            handle_applicationShouldOpenUntitledFile();
        }
        return true
    }
    
    override func applicationDidFinishLaunching(_ aNotification: Notification) {
        launched = true;
        // Logout, restart and shutdown also terminate the app; they are not a
        // user quit, so the device must come back online after reboot.
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.willPowerOffNotification, object: nil, queue: .main
        ) { _ in
            rustdeskMacMarkProgrammaticQuit()
        }
        NSApplication.shared.activate(ignoringOtherApps: true);
    }
}
