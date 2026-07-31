'use strict';

// Aggregator jobs exist only to give branch protection a single required
// check (e.g. "Integration Test L2"); they fail whenever any real job fails,
// so listing them in the alert would only duplicate information. They are
// recognized by their check step rather than by name so that every workflow's
// aggregator is covered regardless of what the job itself is called.
const AGGREGATOR_STEP_NAME = 'Check if any job failed';

const FAILING_JOB_CONCLUSIONS = new Set([
  'failure',
  'timed_out',
  'action_required',
]);

// A step killed by a timeout or a runner-side cancellation reports
// `cancelled` while its job still reports `failure`, so `cancelled` steps
// must be considered to locate where a failed job actually stopped.
const FAILING_STEP_CONCLUSIONS = new Set(['failure', 'timed_out', 'cancelled']);

// Slack section blocks are limited to 3000 characters and each job line can
// take ~200 with its link, so cap the list defensively.
const MAX_LISTED_JOBS = 10;

const CONCLUSION_VERBS = {
  failure: 'failed',
  timed_out: 'timed out',
  cancelled: 'was cancelled',
  action_required: 'needs action',
};

// Escape the characters that are special in Slack mrkdwn text.
function escapeSlackText(text) {
  return String(text)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;');
}

function formatDuration(startedAt, completedAt) {
  if (!startedAt || !completedAt) {
    return null;
  }
  const ms = new Date(completedAt) - new Date(startedAt);
  if (!Number.isFinite(ms) || ms <= 0) {
    return null;
  }
  const totalMinutes = Math.floor(ms / 60_000);
  if (totalMinutes >= 60) {
    return `${Math.floor(totalMinutes / 60)}h ${totalMinutes % 60}m`;
  }
  const seconds = Math.round((ms % 60_000) / 1000);
  if (totalMinutes > 0) {
    return `${totalMinutes}m ${seconds}s`;
  }
  return `${seconds}s`;
}

function firstFailingStep(job) {
  return (job.steps ?? []).find(
    step => step?.conclusion && FAILING_STEP_CONCLUSIONS.has(step.conclusion)
  );
}

function isAggregatorJob(job) {
  return firstFailingStep(job)?.name === AGGREGATOR_STEP_NAME;
}

// Render one Slack mrkdwn bullet per job, e.g.:
// - <https://github.com/...|State Reconstruction Tests> — step `Run tests` was cancelled after 18m 47s
function describeJob(job) {
  const name = escapeSlackText(job.name);
  const link = job.html_url ? `<${job.html_url}|${name}>` : name;
  const step = firstFailingStep(job);
  const verb = CONCLUSION_VERBS[step?.conclusion ?? job.conclusion] ?? 'failed';
  const where = step ? `step \`${escapeSlackText(step.name)}\` ` : '';
  const duration = formatDuration(job.started_at, job.completed_at);
  const after = duration ? ` after ${duration}` : '';
  return `- ${link} — ${where}${verb}${after}`;
}

/**
 * Collects the failed jobs of the triggering workflow run and exposes a
 * Slack mrkdwn summary of them via the `summary` step output.
 * @param {{ github: import('@actions/github').GitHub, core: import('@actions/core'), context: any }} deps
 */
module.exports = async function collectFailedJobs({ github, core, context }) {
  const runId = context.payload.workflow_run.id;
  const attemptNumber = context.payload.workflow_run.run_attempt ?? 1;
  const { owner, repo } = context.repo;

  // Deduplicate by name keeping the latest attempt, since the run-wide
  // fallback listing includes jobs from every attempt.
  const failedJobsByName = new Map();

  function considerJob(job) {
    if (
      !job?.name ||
      !job?.conclusion ||
      !FAILING_JOB_CONCLUSIONS.has(job.conclusion)
    ) {
      return;
    }
    const seen = failedJobsByName.get(job.name);
    if (!seen || (job.run_attempt ?? 1) > (seen.run_attempt ?? 1)) {
      failedJobsByName.set(job.name, job);
    }
  }

  async function collectJobs(fetchPage) {
    let page = 1;
    while (true) {
      const response = await fetchPage(page);

      const jobs = Array.isArray(response?.data?.jobs) ? response.data.jobs : [];
      jobs.forEach(considerJob);

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

  const failedJobs = Array.from(failedJobsByName.values());
  // Fall back to the aggregators only when no real job failed, so the alert
  // always names at least one job.
  let listedJobs = failedJobs.filter(job => !isAggregatorJob(job));
  if (listedJobs.length === 0) {
    listedJobs = failedJobs;
  }

  const lines = listedJobs.slice(0, MAX_LISTED_JOBS).map(describeJob);
  if (listedJobs.length > MAX_LISTED_JOBS) {
    lines.push(`- …and ${listedJobs.length - MAX_LISTED_JOBS} more`);
  }

  core.setOutput('summary', lines.length > 0 ? lines.join('\n') : 'Unknown job');
};
