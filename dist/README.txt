FAMILY TREE  (portable)
=======================

Double-click family-app.exe. Nothing is installed.

FIRST START
  A short welcome screen asks for a name for your tree, an optional password and an optional .ged file to import.
  If you set a password, a RECOVERY KEY is shown once. Print it or save it somewhere safe: without the password
  or the key nobody can open the tree, not even us.

WHERE YOUR DATA IS
  The file named "portable" beside family-app.exe tells the program to keep everything in the folder "data" next
  to it. To move the program and your tree to another computer or a USB stick, copy this whole folder.
  The welcome screen's "Change..." button can keep the tree in another folder instead.
  Delete the "portable" file if you would rather use your Windows user folder (%APPDATA%\FamilyTree).

BACKUP
  Copy the "data" folder (or use Export in the app). A password-protected tree stays protected in the copy.

IF THE WINDOW DOES NOT OPEN
  The window needs the free Microsoft Edge WebView2 Runtime (already part of Windows 11 and updated Windows 10):
  https://developer.microsoft.com/microsoft-edge/webview2/
  Or start the program from a command prompt as:  family-app.exe --browser   (uses your web browser instead).

"WINDOWS PROTECTED YOUR PC"
  The program is not code-signed yet, so Windows may show this warning the first time. Choose "More info", then
  "Run anyway".

AI ASSISTANTS
  Assistants can read the tree and PROPOSE additions; nothing changes until you approve it in the Review screen.
  The AI assistant panel (key icon) shows how to connect one.

COMMAND-LINE OPTIONS
  --browser            open in the default browser instead of the window
  --data <folder>      keep the data in this folder
  --headless           only run the local service (for tools and tests)
