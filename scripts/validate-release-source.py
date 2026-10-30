"""Gate app artifacts, including a completed app job in the current release run."""
import json
import os
import re
import subprocess
import sys


def validate(run, jobs, artifacts, *, repository, current_run, current_sha):
    if (run.get('repository') or {}).get('full_name') != repository:
        raise ValueError('Application run repository mismatch')
    if (run.get('head_repository') or {}).get('full_name') != repository:
        raise ValueError('Application source repository mismatch')
    sha = run.get('head_sha', '')
    if not re.fullmatch('[0-9a-f]{40}', sha):
        raise ValueError('Application source commit is invalid')
    if run.get('conclusion') != 'success':
        # A reusable firmware job runs after app but before its outer run can
        # finish. Only this exact run/commit may use the narrower app-job gate.
        if (str(run.get('id')) != str(current_run) or sha != current_sha
                or run.get('status') != 'in_progress'):
            raise ValueError('Application run has not succeeded')
        app = [job for job in jobs if job.get('name') == 'app']
        if len(app) != 1 or app[0].get('status') != 'completed' or app[0].get('conclusion') != 'success':
            raise ValueError('Current application job has not succeeded')
    for name in ('carlink-armv7', 'carplay-armv7'):
        matches = [a for a in artifacts if a.get('name') == name and not a.get('expired')]
        if (len(matches) != 1 or not matches[0].get('size_in_bytes', 0)
                or not re.fullmatch('sha256:[0-9a-f]{64}', matches[0].get('digest', ''))):
            raise ValueError('Missing verified application artifact: ' + name)
    return sha


def main():
    run_id = sys.argv[1]
    if not run_id.isdecimal():
        raise ValueError('Invalid application run id')
    repository = os.environ['GITHUB_REPOSITORY']
    def api(suffix):
        result = subprocess.run(['gh', 'api', f'repos/{repository}/actions/runs/{run_id}{suffix}'],
                                check=True, text=True, capture_output=True)
        return json.loads(result.stdout)
    run = api('')
    jobs = api('/jobs?per_page=100')['jobs'] if run.get('conclusion') != 'success' else []
    artifacts = api('/artifacts?per_page=100')['artifacts']
    sha = validate(run, jobs, artifacts, repository=repository,
                   current_run=os.environ['GITHUB_RUN_ID'], current_sha=os.environ['GITHUB_SHA'])
    print('APP_COMMIT=' + sha)


if __name__ == '__main__':
    main()
