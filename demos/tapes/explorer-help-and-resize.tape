Output demos/out/explorer-help-and-resize.gif
Output demos/out/explorer-help-and-resize.mp4

Require cargo
Require python3
Set Shell bash
Set Width 1600
Set Height 660
Set FontSize 22
Set TypingSpeed 25ms
Set Theme "Catppuccin Mocha"

Hide
Type "cargo build -q --locked -p jgrep"
Enter
Sleep 2s
Show

Type "python3 demos/scripts/explorer_recording.py --narrow -- -f name demos/fixtures/users.ndjson"
Enter
Wait+Screen /F1 help/
Sleep 2s

# VHS 0.11 has no function-key command. The adapter translates this to F1.
Ctrl+R
Wait+Screen /Explorer help/
Wait+Screen /F1\/Esc close help/
Sleep 3s
PageDown
Wait+Screen /Ctrl-S: save/
Sleep 3s
Escape
Wait+Screen /Alice/
Sleep 2s

# Expand the actual app PTY to 100 columns and restore the Fields panel.
Ctrl+T
Wait+Screen /Fields/
Sleep 3s
Enter
Wait+Screen /Carla/
Sleep 2s
