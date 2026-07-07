Output demos/out/counts-files-null.gif

Require cargo

Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 22
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Type "cargo build -q -p jgrep"
Enter
Sleep 2s

Type "./target/debug/jgrep -c -w status=active demos/fixtures/users.ndjson"
Enter
Wait+Screen /2/
Sleep 1s

Type "./target/debug/jgrep -l -w status=active demos/fixtures/users.ndjson demos/fixtures/logs.ndjson"
Enter
Wait+Screen /users.ndjson/
Sleep 1s

Type "./target/debug/jgrep -n '{count:2, files:1, null_input:true}'"
Enter
Wait+Screen /null_input/
Sleep 2s
