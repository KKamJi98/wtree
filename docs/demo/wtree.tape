# Run from the repository root with: bash docs/demo/record.sh
# Actual wt 2.0.1 release output, fake data, local remote, no network or host mounts.
Output docs/demo/wtree.gif
Output docs/demo/wtree.mp4
Require docker
Set Shell "bash"
Set Width 1200
Set Height 640
Set FontFamily "Menlo"
Set FontSize 17
Set LineHeight 1.15
Set Padding 24
Set Theme "Builtin Light"
Set Framerate 15
Set PlaybackSpeed 1.5
Set TypingSpeed 55ms
Set WindowBar Colorful
Set WindowBarSize 32
Set BorderRadius 10
Set Margin 12
Set MarginFill "#ffffff"
Hide
Type "docker run --rm -it --network none --hostname demo wtree-readme-demo:local"
Enter
Sleep 3s
Wait+Line@30s /^\$$/
Ctrl+L
Show
Type "wt status"
Enter
Wait+Screen /Summary: total=4 clean=3 dirty=1/
Sleep 4s
Ctrl+L
Type "wt pull"
Enter
Wait+Screen@30s /Summary: ok=3 skip=1 fail=0/
Sleep 4s
Ctrl+L
Type "wt add feat/search -c"
Enter
Wait+Screen@30s /git push -u origin feat\/search/
Sleep 2s
Ctrl+L
Type "wt rm feat/search -y"
Enter
Wait+Screen@30s /Summary: ok=1 skip=0 kept=0 fail=0/
Sleep 2s
Ctrl+L
Type "wt status"
Enter
Wait+Screen /Summary: total=4 clean=3 dirty=1/
Sleep 4s
