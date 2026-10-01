Output demos/out/explorer-complete-save-export.gif
Output demos/out/explorer-complete-save-export.mp4

Require cargo
Require python3
Require cat
Set Shell bash
Set Width 1600
Set Height 1000
Set FontSize 22
Set TypingSpeed 25ms
Set Theme "Catppuccin Mocha"

Hide
Type "cargo build -q --locked -p jgrep"
Enter
Sleep 2s
Show

Type "python3 demos/scripts/explorer_recording.py -- -f status=active demos/fixtures/users.ndjson"
Enter
Wait+Screen /2 matches/
Ctrl+O
Type "na"
Tab
Wait+Screen /Alice/
Wait+Screen /Carla/
Sleep 2s
Ctrl+S
Wait+Screen /saved/
Sleep 2s

# Recall the saved filter through history, then export the generated jq.
Ctrl+O
Backspace 13
Type "status=pending"
Wait+Screen /Bob/
Sleep 2s
Ctrl+P
Wait+Screen /status=active/
Sleep 2s
Ctrl+Y
Wait+Screen /select\(.status == "active"\) \| .name/
Sleep 2s

Type "cat demos/out/explorer-filter.jq"
Enter
Wait+Screen /select\(.status == "active"\) \| .name/
Sleep 2s
