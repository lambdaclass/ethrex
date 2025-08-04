module.exports = async ({ github, context }) => {
    // Gets a project given the number and the organization.
    async function getProject(org, number) {
        const res = await github.graphql(`
        query($org: String!) {
            organization(login: $org) {
            projectsV2(first: 100) {
                nodes {
                id
                title
                number
                }
            }
            }
        }
        `, { org });
        const project = res.organization.projectsV2.nodes.find(p => p.number === number);
        if (!project) throw new Error(`Project #${number} not found in org ${org}`);
        return project;
    }

    // Find an Issue or PR item for a specific project.
    async function findItemInProject(owner, repo, itemNumber, projectId) {
        const res = await github.graphql(`
        query($owner: String!, $repo: String!, $itemNumber: Int!) {
            repository(owner: $owner, name: $repo) {
                issueOrPullRequest(number: $itemNumber) {
                    ... on PullRequest {
                        projectItems(first: 10) {
                            nodes { id project { id } }
                        }
                    }
                    ... on Issue {
                        projectItems(first: 10) {
                            nodes { id project { id } }
                        }
                    }
                }
            }
        }
        `, {
            owner,
            repo,
            itemNumber
        });

        const projectItems = res.repository?.issueOrPullRequest?.projectItems?.nodes;

        if (!projectItems) {
            return null;
        }

        const item = projectItems.find(p => p.project.id === projectId);
        return item || null;
    }

    // Gets all fields of a project. Any kind of field.
    async function getProjectFields(projectId) {
        const res = await github.graphql(`
        query($projectId: ID!) {
            node(id: $projectId) {
            ... on ProjectV2 {
                fields(first: 100) {
                nodes {
                    __typename
                    ... on ProjectV2SingleSelectField {
                    id
                    name
                    options {
                        id
                        name
                    }
                    }
                    ... on ProjectV2IterationField {
                    id
                    name
                    configuration {
                        iterations {
                        id
                        title
                        startDate
                        duration
                        }
                    }
                    }
                    ... on ProjectV2FieldCommon {
                    id
                    name
                    }
                }
                }
            }
            }
        }
        `, { projectId });

        // Return an array of all fields with their options if available
        return res.node.fields.nodes;
    }


    // Sets a date field.
    async function setProjectItemDateField(projectId, itemId, field, dateValue) {
        const date = dateValue.split("T")[0]; // "YYYY-MM-DD"

        await github.graphql(`
        mutation {
            updateProjectV2ItemFieldValue(input: {
            projectId: "${projectId}",
            itemId: "${itemId}",
            fieldId: "${field.id}",
            value: { date: "${date}" }
            }) {
            projectV2Item { id }
            }
        }
        `);

        console.log(`Set date field '${field.name}' to '${date}'.`);
    }

    // Sets the field for status to a project item (PR or Issue). E.g., Set Status for PR #100 to "In Progress".
    async function setProjectItemStatus(itemId, field, optionName, projectId) {
        // Ensure the field is a single select and has options
        if (!field.options) throw new Error(`Field '${field.name}' has no options`);

        // Find the matching option
        const option = field.options.find(opt => opt.name === optionName);
        if (!option) throw new Error(`Option '${optionName}' not found in field '${field.name}'`);

        // Run the mutation
        await github.graphql(`
        mutation($projectId: ID!, $itemId: ID!, $fieldId: ID!, $optionId: String!) {
            updateProjectV2ItemFieldValue(input: {
            projectId: $projectId,
            itemId: $itemId,
            fieldId: $fieldId,
            value: { singleSelectOptionId: $optionId }
            }) {
            projectV2Item { id }
            }
        }
        `, {
            projectId,
            itemId,
            fieldId: field.id,
            optionId: option.id
        });
    }

    // Gets all issue numbers that would be closed if the PR is merged.
    function extractLinkedIssueNumbers(prBody) {
        const body = prBody || "";
        const withoutComments = body.replace(/<!--[\s\S]*?-->/g, "");
        const matches = [...withoutComments.matchAll(/(?:close[sd]?|fixe[sd]?|resolve[sd]?)\s+#(\d+)/gi)];
        return matches.map(match => parseInt(match[1], 10));
    }

    function findItemByNumber(allItems, number) {
        return allItems.find(i => i.content?.number === number);
    }

    function getFieldByName(fields, fieldName) {
        const field = fields.find(f => f.name === fieldName);
        if (!field) throw new Error(`Field '${fieldName}' not found`);
        return field;
    }

    // ========== MAIN LOGIC ==========
    const pr = context.payload.pull_request;

    // Run the status check only if the PR contains the l1 label.
    // This check is for the submitted review, the check for a pull_request event was previously done.
    if (context.eventName === "pull_request_review") {
        const prNumber = pr.number;
        const { data: labels } = await github.rest.issues.listLabelsOnIssue({
            owner: context.repo.owner,
            repo: context.repo.repo,
            issue_number: prNumber,
        });

        const hasL1 = labels.some(label => label.name === "L1");
        if (!hasL1) {
            console.log("PR does not have 'L1' label. Exiting.");
            return;
        } else {
            console.log("PR contains 'L1' label. Continuing with status checks...")
        }
    }

    const projectNumber = 31;
    const orgLogin = "lambdaclass";
    const repo = "ethrex";

    // Get project and item representing Pull Request. Exiting early if it doesn't belong to the project.
    const project = await getProject(orgLogin, projectNumber);
    const projectId = project.id;

    const prItem = await findItemInProject(orgLogin, repo, pr.number, projectId);
    if (!prItem) {
        console.warn(`PR #${pr.number} not found in ethrex_l1 project. Exiting...`);
        return;
    }

    // Get all fields of the project.
    const fields = await getProjectFields(projectId);

    const statusField = getFieldByName(fields, "Status");
    const lastUpdatedField = getFieldByName(fields, "Last updated");
    const startDateField = getFieldByName(fields, "Start date");

    const action = context.payload.action;

    // Set Start date if PR was opened
    if (action === "opened") {
        await setProjectItemDateField(projectId, prItem.id, startDateField, pr.created_at);
    }

    // Set date of Last update only if it's an update to the PR
    if (action === "synchronize" || action === "edited") {
        await setProjectItemDateField(projectId, prItem.id, lastUpdatedField, pr.updated_at);
    }

    const isDraftTransition = (action === "opened" || action === "reopened") && pr.draft || action === "converted_to_draft";
    const readyForReview = action === "ready_for_review";
    const requestedChanges = context.eventName === "pull_request_review" && context.payload.review?.state === "changes_requested";

    // Determine new status of the PR.
    let newStatus = null;

    if (isDraftTransition || requestedChanges) {
        newStatus = "In Progress";
    } else if (readyForReview || ((action === "opened" || action === "reopened") && !pr.draft)) {
        newStatus = "In Review";
    }

    if (!newStatus) {
        console.log("No status change required. Exiting.");
        return;
    }

    // Set new status of the PR
    console.log(`Setting new status of the PR to '${newStatus}'`);
    await setProjectItemStatus(prItem.id, statusField, newStatus, projectId);

    // Set new status of each issue linked to the PR
    const issueNumbers = extractLinkedIssueNumbers(pr.body);
    console.log("Issues that this PR closes:", issueNumbers);

    console.log(`Synchronizing linked issues with new PR status.`);
    for (const issueNumber of issueNumbers) {
        const issueItem = await findItemInProject(orgLogin, repo, issueNumber, projectId);

        if (issueItem) {
            console.log(`Setting status of issue #${issueNumber} to '${newStatus}'.`);
            await setProjectItemStatus(issueItem.id, statusField, newStatus, projectId);
        } else {
            console.warn(`Project item for issue #${issueNumber} not found.`);
        }
    }
}
