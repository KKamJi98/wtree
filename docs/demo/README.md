# Reproduce the wtree demo

From the repository root:

```bash
bash docs/demo/record.sh
```

Requirements: Docker (running), VHS, ttyd, ffmpeg, gifsicle, curl, shasum,
and tar. Set `TMPDIR` to a scratch directory. The tape uses Menlo; change
`FontFamily` to a locally installed monospace font when recording on Linux.
The checked-in recording uses the checksummed **wt 2.0.1** Linux release.
It is 26.44 seconds at 1200 x 640 pixels. The optimized GIF is 191,451 bytes;
the MP4 is 229,712 bytes. These are artifact sizes, not a runtime benchmark.
The tape waits for actual completion messages before proceeding; terminal clears
keep each action within one viewport. Playback timing is set in the tape.

`record.sh` downloads the release and builds a small Alpine image. The recording
then runs with `--network none`, a neutral hostname, no host directory mounts,
and a fresh `/demo/home`. `setup.sh` creates fake AWS/SSH config, a local bare
remote, and four worktrees. No real config, credentials, repositories, hosts,
or user identity enter that container. The script refuses to run directly on
the host. Re-running it uses a new disposable container, not existing data.

The commands on screen execute the unmodified binary. The only edits are
terminal clears, timing, and GIF compression; no command output is substituted.
`status` and `pull` return 2 here because the login worktree is dirty. That is
expected. After the pull, two branches fast-forward, the dirty branch is
skipped, and a temporary worktree is created and removed without deleting its
branch. The final status stays visible before the loop restarts.

Outputs: `wtree.gif` for the README and `wtree.mp4` for playback. Before
publishing a new recording, extract frames with ffmpeg, inspect every scene
(including command transitions), and confirm all visible paths start at `/demo`.
