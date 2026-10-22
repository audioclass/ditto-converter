"""Exact source-variant regression tests; uses the real checksum-pinned archive."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'scripts/prepare-remediation.py'


class RemediationTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), 'Hash-verified remediation preparation is not implemented')
        spec = importlib.util.spec_from_file_location('remediation', SCRIPT)
        assert spec is not None and spec.loader is not None
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)
        scratch = ROOT / '.work/remediation-tests'
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        shutil.copytree(ROOT / 'scripts/remediation', self.root / 'scripts/remediation')
        self.archive = Path(os.environ.get('DITTO_REMEDIATION_TEST_ARCHIVE', ROOT / '.work/dependency-remediation/archive.crate'))

    def prepare(self):
        return self.module.prepare(self.root, archive_path=self.archive)

    def test_exactly_one_file_replaced_and_every_other_archive_byte_preserved(self):
        result = self.prepare()
        recipe = json.loads((self.root / 'scripts/remediation/recipe.json').read_text())
        base = self.root / recipe['output']
        with tarfile.open(self.archive) as archive:
            changed = []
            for member in archive.getmembers():
                if not member.isfile():
                    continue
                relative = '/'.join(member.name.split('/')[1:])
                stream = archive.extractfile(member)
                assert stream is not None
                before = stream.read()
                after = (base / relative).read_bytes()
                if after != before:
                    changed.append(relative)
            self.assertEqual(changed, ['src/user_notification.rs'])
        self.assertEqual(hashlib.sha256((base / recipe['file']).read_bytes()).hexdigest(), recipe['replacement_sha256'])
        self.assertEqual(result['archive_sha256'], recipe['registry_archive_sha256'])
        self.assertNotIn(b'pub ', (base / recipe['file']).read_bytes())
        self.assertEqual(self.prepare(), result)

    def test_tampered_archive_rejected_before_extract(self):
        bad = self.root / 'bad.crate'
        bad.write_bytes(self.archive.read_bytes() + b'tampered')
        with self.assertRaisesRegex(RuntimeError, 'archive.*SHA256'):
            self.module.prepare(self.root, archive_path=bad)

    def test_tampered_replacement_and_recipe_rejected(self):
        replacement = self.root / 'scripts/remediation/user_notification.rs'
        original = replacement.read_bytes()
        replacement.write_bytes(b'changed')
        with self.assertRaisesRegex(RuntimeError, 'replacement.*SHA256'):
            self.prepare()
        replacement.write_bytes(original)
        recipe = self.root / 'scripts/remediation/recipe.json'
        recipe.write_bytes(recipe.read_bytes() + b' ')
        with self.assertRaisesRegex(RuntimeError, 'recipe.*SHA256'):
            self.prepare()

    def test_tampered_reconstructed_file_and_extra_file_rejected(self):
        self.prepare()
        recipe = json.loads((self.root / 'scripts/remediation/recipe.json').read_text())
        base = self.root / recipe['output']
        victim = base / 'Cargo.toml'
        original = victim.read_bytes()
        victim.write_bytes(original + b'\n# tampered\n')
        with self.assertRaisesRegex(RuntimeError, 'reconstructed'):
            self.prepare()
        victim.write_bytes(original)
        (base / 'extra.rs').write_bytes(b'extra')
        with self.assertRaisesRegex(RuntimeError, 'reconstructed'):
            self.prepare()

    def test_symlink_output_rejected(self):
        if os.name == 'nt':
            self.skipTest('Windows symlink creation needs additional privileges')
        self.prepare()
        recipe = json.loads((self.root / 'scripts/remediation/recipe.json').read_text())
        base = self.root / recipe['output']
        victim = base / 'Cargo.toml'
        original = victim.read_bytes()
        target = self.root / 'manifest'
        target.write_bytes(original)
        victim.unlink()
        victim.symlink_to(target)
        with self.assertRaisesRegex(RuntimeError, 'symlink|reconstructed'):
            self.prepare()


if __name__ == '__main__':
    unittest.main()
