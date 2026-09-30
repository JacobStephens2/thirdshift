Write the summary of a thirdshift release for its release pull request and its GitHub Release page. Below this prompt are the release's version diff and the title, number and body of each pull request merged since the last release.

Print only the summary, in GitHub Markdown, with no heading and nothing before or after it:

1. A short headline paragraph on what the release is about: its biggest change for someone running `thirdshift`, in a sentence or two.
2. What changes for someone running `thirdshift`, first: new commands, then behaviour that is new or changed, most important first.
3. Site, docs and CI work in one short grouped line after that, such as "Also: site, docs and CI updates (#12, #14, #15)." Leave it out if there was none.

Reference every change by its pull request number, as `#<number>`. Several pull requests that make one change can share a bullet.

Claim nothing the pull request titles and bodies don't say. Don't guess at motives, impact or details they leave out. When a title is all there is, say no more than the title does.

Aim for the tone and density of the 0.3.0 summary:

> 0.3.0 ships the changes merged since v0.2.0. The headline is the **Merge run** (`thirdshift merge <Issue URL>`): #89 (define Merge run, Self-merge and Foreign commit), #96 (tracer bullet), #97 (a failed Self-merge goes back round the Repair loop; policy refusals leave the PR ready), #98 (post-merge steps: delete the Issue branch, close the issue), #99 (review Foreign commits with a Repair). Also #95 (Repair and base-move budgets raised to 5) and the site work in #72–#87.
