"""Pandas runner for an Ohmypipeline document.

The window never imports this package. `ompp` speaks length-prefixed JSON
on stdin and stdout. Warnings and tracebacks stay on stderr.
"""

__all__ = ["run_job"]

from .run import run_job
