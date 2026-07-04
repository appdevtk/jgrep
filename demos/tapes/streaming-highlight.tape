Output demos/out/streaming-highlight.gif

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

Type "JGREP_DEMO_DELAY=0.35 demos/scripts/k8s-log-stream.sh | ./target/debug/jgrep -C -f demos/fixtures/k8s-log-line.jq"
Enter
Wait+Screen /payment declined/
Wait+Screen /fallback charge failed/
Sleep 2s
