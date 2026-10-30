"""Release source gates: reject unrelated/failed/incomplete current-run sources."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('release', Path(__file__).with_name('validate-release-source.py'))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)

class SourceTests(unittest.TestCase):
    def setUp(self):
        self.sha = 'a' * 40
        self.run = dict(id=12, status='in_progress', conclusion=None, head_sha=self.sha,
                        repository=dict(full_name='owner/project'), head_repository=dict(full_name='owner/project'))
        self.jobs = [dict(name='app', status='completed', conclusion='success')]
        self.artifacts = [dict(name=name, expired=False, size_in_bytes=12, digest='sha256:'+'b'*64)
                          for name in ('carlink-armv7', 'carplay-armv7')]
    def check(self):
        return release.validate(self.run, self.jobs, self.artifacts, repository='owner/project', current_run='12', current_sha=self.sha)
    def test_completed_app_and_uploaded_artifacts_allow_current_run(self):
        self.assertEqual(self.check(), self.sha)
    def test_other_in_progress_run_is_rejected(self):
        self.run['id'] = 13
        with self.assertRaisesRegex(ValueError, 'has not succeeded'):self.check()
    def test_wrong_commit_is_rejected_for_current_run(self):
        self.run['head_sha'] = 'c' * 40
        with self.assertRaises(ValueError):self.check()
    def test_failed_running_skipped_or_duplicate_app_job_is_rejected(self):
        for jobs in ([], [dict(name='app',status='in_progress',conclusion=None)],
                     [dict(name='app',status='completed',conclusion='failure')],
                     [dict(name='app',status='completed',conclusion='skipped')], self.jobs*2):
            self.jobs = jobs
            with self.assertRaises(ValueError):self.check()
    def test_unrelated_repository_is_rejected(self):
        for key in ('repository', 'head_repository'):
            old = self.run[key]
            self.run[key] = dict(full_name='fork/project')
            with self.assertRaises(ValueError):self.check()
            self.run[key] = old
    def test_missing_expired_unhashed_duplicate_or_empty_artifact_is_rejected(self):
        for change in ('missing','expired','unhashed','duplicate','empty'):
            original = copy.deepcopy(self.artifacts)
            if change=='missing':self.artifacts.pop()
            if change=='expired':self.artifacts[0]['expired']=True
            if change=='unhashed':self.artifacts[0]['digest']=''
            if change=='duplicate':self.artifacts.append(copy.deepcopy(self.artifacts[0]))
            if change=='empty':self.artifacts[0]['size_in_bytes']=0
            with self.assertRaises(ValueError):self.check()
            self.artifacts = original
    def test_successful_previous_run_is_allowed(self):
        self.run.update(id=1,status='completed',conclusion='success',head_sha='d'*40)
        self.jobs=[]
        self.assertEqual(self.check(),'d'*40)
    def test_failed_previous_run_is_rejected(self):
        self.run.update(id=1,status='completed',conclusion='failure')
        with self.assertRaises(ValueError):self.check()

if __name__=='__main__':unittest.main()
