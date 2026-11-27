from pathlib import Path

from workdir import root


def test_root_comes_from_the_environment_when_set():
    assert root({"ROUTE96_TAXONOMY_ROOT": "/tmp/r96"}) == Path("/tmp/r96")


def test_root_defaults_to_a_folder_in_home_outside_any_worktree():
    assert root({}) == Path.home() / "route96-taxonomy"
