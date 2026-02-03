'use strict';

/**
 * Collects failed job names for the current workflow run and exposes them via step output.
 * @param {{ github: import('@actions/github').GitHub, core: import('@actions/core'), context: any }} deps
 */
module.exports = async function collectFailedJobs({ github, core, context }) {
  const runId = context.payload.workflow_run.id;
  const attemptNumber = context.payload.workflow_run.run_attempt ?? 1;
  const { owner, repo } = context.repo;

  const failingConclusions = new Set(['failure', 'timed_out', 'action_required']);
  const ignoredJobs = new Set(['Integration Test']);
  const relevantJobs = new Set();

  async function collectJobs(fetchPage) {
    let page = 1;
    while (true) {
      const response = await fetchPage(page);

      const jobs = Array.isArray(response?.data?.jobs) ? response.data.jobs : [];
      for (const job of jobs) {
        if (
          job?.conclusion &&
          failingConclusions.has(job.conclusion) &&
          job?.name &&
          !ignoredJobs.has(job.name)
        ) {
          relevantJobs.add(job.name);
        }
      }

      if (jobs.length < 100) {
        break;
      }
      page += 1;
    }
  }

  try {
    await collectJobs(page =>
      github.rest.actions.listJobsForWorkflowRunAttempt({
        owner,
        repo,
        run_id: runId,
        attempt_number: attemptNumber,
        per_page: 100,
        page,
      })
    );
  } catch (error) {
    if (error?.status !== 404) {
      throw error;
    }
    core.info('Falling back to run-wide job listing');
    await collectJobs(page =>
      github.rest.actions.listJobsForWorkflowRun({
        owner,
        repo,
        run_id: runId,
        per_page: 100,
        page,
      })
    );
  }

  const jobList = Array.from(relevantJobs);
  const names = jobList.length > 0 ? jobList.join('\n- ') : 'Unknown job';
  core.setOutput('names', jobList.length > 0 ? `- ${names}` : names);
};
