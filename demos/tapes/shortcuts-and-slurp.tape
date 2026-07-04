Output demos/out/shortcuts-and-slurp.gif

Require cargo

Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 24
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Type "cargo build -q -p jgrep"
Enter
Sleep 2s

Type "./target/debug/jgrep -w status=active -p name demos/fixtures/users.ndjson"
Enter
Wait+Screen /Carla/
Sleep 1s

Type "./target/debug/jgrep -f demos/fixtures/filter.jq demos/fixtures/users.ndjson"
Enter
Wait+Screen /Berlin/
Sleep 1s

Type "./target/debug/jgrep -s -p role demos/fixtures/users.ndjson"
Enter
Wait+Screen /operator/
Sleep 2s
