Output demos/out/shortcuts-and-slurp.gif

Require cargo

Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 24
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Type "cargo build -q -p jgrep && alias jgrep=./target/debug/jgrep"
Enter
Sleep 2s

Type "jgrep -w status=active -p name demos/fixtures/users.ndjson"
Enter
Wait+Line /Carla/
Sleep 1s

Type "jgrep -f demos/fixtures/filter.jq demos/fixtures/users.ndjson"
Enter
Wait+Line /Berlin/
Sleep 1s

Type "jgrep -s '.role' demos/fixtures/users.ndjson"
Enter
Wait+Line /operator/
Sleep 2s
