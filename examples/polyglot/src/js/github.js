// @openshell egress api.github.com:443 reason="release and issue metadata" access=read-only scope=file
// @openshell provider GITHUB_TOKEN profile=github

async function latest(repo) {
  const res = await fetch(`https://api.github.com/repos/${repo}/releases/latest`, {
    headers: { Authorization: `Bearer ${process.env.GITHUB_TOKEN}` },
  });
  return res.json();
}

async function issues(repo) {
  const res = await fetch(`https://api.github.com/repos/${repo}/issues`);
  return res.json();
}

// @openshell ignore kind=net reason="webhook host is chosen by the operator at runtime"
async function notify(webhookUrl, body) {
  await fetch(webhookUrl, { method: "POST", body });
}

// @openshell ignore kind=exec reason="local tar listing, no network"
function listArchive() {
  const { execFileSync } = require("child_process");
  return execFileSync("tar", ["-tzf", "/tmp/tools.tar.gz"]);
}

// A string is not a comment, so the marker below is not a directive.
const hidden = "// @openshell egress not-a-directive.test:443 reason=\"inside a string\"";

module.exports = { latest, issues, notify, listArchive, hidden };
