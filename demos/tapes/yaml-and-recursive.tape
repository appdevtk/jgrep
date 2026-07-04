Output demos/out/yaml-and-recursive.gif

Require cargo

Set Shell bash
Set Width 1200
Set Height 720
Set FontSize 24
Set TypingSpeed 35ms
Set Theme "Catppuccin Mocha"

Type "cargo build -q -p jgrep"
Enter
Sleep 2s

Type "./target/debug/jgrep -f demos/fixtures/images.jq demos/fixtures/k8s/deployment.yaml"
Enter
Wait+Screen /checkout:1.7.4/
Sleep 1s

Type "./target/debug/jgrep -r -f demos/fixtures/deployments.jq demos/fixtures/k8s"
Enter
Wait+Screen /checkout/
Sleep 2s
