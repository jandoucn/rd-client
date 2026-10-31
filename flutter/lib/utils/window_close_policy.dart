/// Whether closing the last visible window should terminate the process.
///
/// On macOS the GUI process may host the server itself (installs without the
/// launchd daemon), so closing the window only hides it and the device stays
/// online; the user quits explicitly from the Dock or the app menu.
bool quitWhenAllWindowsClosed({required bool isMacOS}) => !isMacOS;
