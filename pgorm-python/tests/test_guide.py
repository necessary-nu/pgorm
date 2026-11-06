"""The Python guide: its chapters follow the Rust guide's, and its examples run.

The guide is ``pgorm-python/guide``. Its chapter list is held to
``docs/guide/chapters.json`` here, and every ```python block in it runs as a
program of its own, in a scratch database holding the bakery, on the server
``PGORM_TEST_DSN`` names, or else ``DATABASE_URL``, as ``examples.test_server``
reads it. ``PGORM_GUIDE_CHAPTERS``, a comma-separated list of chapter ids,
narrows the run to those chapters.
"""

import asyncio
import os
from pathlib import Path
import re
import sys
import unittest

GUIDE = Path(__file__).resolve().parents[1] / "guide"
sys.path.insert(0, str(GUIDE))

import examples  # noqa: E402

CHAPTER_FILE = re.compile(r"\d\d-[a-z0-9-]+\.md")


class GuideStructure(unittest.TestCase):
    # [spec:pgorm:req:guide.dialects/test]    every Rust chapter, under its id and in its order, and only declared extras
    def test_chapters_follow_the_rust_guide(self):
        chapters = examples.chapters()
        shared = [chapter.id for chapter in chapters if not chapter.python_only]
        rust = [chapter["id"] for chapter in examples.rust_chapters()]
        self.assertEqual(shared, rust, "the Python guide's shared chapters are the Rust guide's, in order")
        extra = [chapter.id for chapter in chapters if chapter.python_only]
        self.assertFalse(set(extra) & set(rust), "a Python-only chapter takes an id of its own")
        ids = [chapter.id for chapter in chapters]
        self.assertEqual(len(ids), len(set(ids)), "each chapter is listed once")

    # [spec:pgorm:req:guide.dialects/test]    each listed file exists, is numbered in order and opens with its title
    def test_files_titles_and_numbering(self):
        chapters = examples.chapters()
        for number, chapter in enumerate(chapters, 1):
            with self.subTest(chapter=chapter.id):
                self.assertEqual(chapter.file, f"{number:02d}-{chapter.id}.md")
                text = (GUIDE / chapter.file).read_text()
                self.assertEqual(text.splitlines()[0], f"# {chapter.title}")
        listed = {chapter.file for chapter in chapters}
        on_disk = {path.name for path in GUIDE.glob("*.md") if CHAPTER_FILE.fullmatch(path.name)}
        self.assertEqual(on_disk, listed, "every chapter file is listed, and no other")

    # [spec:pgorm:req:guide.dialects/test]    the index's table holds one row per chapter, in reading order
    def test_index_links_every_chapter_in_order(self):
        rows = [line for line in (GUIDE / "README.md").read_text().splitlines() if line.startswith("| ")]
        linked = [link for row in rows for link in re.findall(r"\]\((\d\d-[a-z0-9-]+\.md)\)", row)[:1]]
        self.assertEqual(linked, [chapter.file for chapter in examples.chapters()])

    # [spec:pgorm:req:guide.dialects/test]    every example is a whole program that asserts what it shows
    def test_every_example_asserts(self):
        for chapter in examples.chapters():
            for example in examples.examples(chapter):
                with self.subTest(chapter=chapter.id, line=example.line):
                    self.assertRegex(example.code, r"\bassert\b",
                                     "an example asserts what it shows")

    # [spec:pgorm:req:guide.dialects/test]    a chapter's links to another chapter name a file that exists
    def test_links_between_chapters_resolve(self):
        files = {path.name for path in GUIDE.iterdir()}
        for chapter in examples.chapters():
            text = (GUIDE / chapter.file).read_text()
            for target in re.findall(r"\]\(([^)#:\s]+)(?:#[^)]*)?\)", text):
                with self.subTest(chapter=chapter.id, target=target):
                    if target.startswith("../") or target.startswith("/"):
                        self.assertTrue((GUIDE / target).resolve().exists(), target)
                    else:
                        self.assertIn(target, files)


class GuideExamples(unittest.TestCase):
    # [spec:pgorm:req:guide.dialects/test]    every example runs against the server, each in a bakery of its own
    def test_every_example_runs(self):
        only = os.environ.get("PGORM_GUIDE_CHAPTERS")
        selected = examples.all_examples(
            registered=False, only=set(only.split(",")) if only else None
        )
        self.assertTrue(selected, "the guide has examples to run")
        server = examples.test_server()
        if server is None:
            self.fail("PGORM_TEST_DSN or DATABASE_URL names the server; tests/with_postgres.py sets one")
        outcomes = asyncio.run(examples.run_all(selected, server))
        for outcome in outcomes:
            example = outcome.example
            with self.subTest(chapter=example.chapter, example=example.index, line=example.line):
                self.assertEqual(outcome.returncode, 0, outcome.output)


if __name__ == "__main__":
    unittest.main()
