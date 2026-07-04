Output demos/out/explore-tui-streaming.gif

Require cargo
Require bash

Set Shell bash
Set Width 1200
Set Height 760
Set FontSize 22
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Type "cargo build -q -p jgrep"
Enter
Sleep 2s

Type "JGREP_DEMO_DELAY=0.45 demos/scripts/k8s-log-stream.sh | ./target/debug/jgrep explore"
Enter
Wait+Screen /streaming stdin/
Sleep 1s

Type "log.level=ERROR"
Wait+Screen /declined/
Wait+Screen /fallback/
Sleep 2s

Escape
Sleep 1s
