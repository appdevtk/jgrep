Output demos/out/cli-entry-and-recovery.gif
Output demos/out/cli-entry-and-recovery.mp4

Require cargo
Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 22
Set TypingSpeed 25ms
Set Theme "Catppuccin Mocha"

Hide
Type "cargo build -q --locked -p jgrep"
Enter
Sleep 2s
Show

# A file alone defaults to identity, no jq knowledge required.
Type "./target/debug/jgrep demos/fixtures/users.ndjson"
Enter
Wait+Screen /Carla/
Sleep 2s

# Explicit filter plus a shortcut condition avoids positional ambiguity.
Type "./target/debug/jgrep --filter '.name' -w status=active demos/fixtures/users.ndjson"
Enter
Wait+Screen /Carla/
Sleep 2s

# Report missing inputs instead of silently waiting for stdin.
Type "./target/debug/jgrep -p name demos/fixtures/missing.json"
Enter
Wait+Screen /No such file/
Sleep 2s

Type "./target/debug/jgrep -p name demos/fixtures/users.ndjson"
Enter
Wait+Screen /Bob/
Sleep 2s
