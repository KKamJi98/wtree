"""wt - Git worktree management CLI tool."""

from importlib.metadata import PackageNotFoundError, version

# Read from the installed distribution rather than repeating the number here.
# The hardcoded copy said 1.1.0 while pyproject said 1.2.0, and nothing was ever
# going to notice: a second source of truth for a version only drifts.
try:
    __version__ = version("wtree")
except PackageNotFoundError:  # running from a source tree with nothing installed
    __version__ = "0.0.0+unknown"
