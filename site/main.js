// Copy buttons: put the command next to the button on the clipboard.
for (const button of document.querySelectorAll(".install button.copy")) {
  const code = button.closest(".install").querySelector("code");
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(code.textContent);
      button.textContent = "Copied";
    } catch {
      // No clipboard access (e.g. an insecure origin): select the command so it can be copied by hand.
      getSelection().selectAllChildren(code);
      button.textContent = "Selected";
    }
    setTimeout(() => { button.textContent = "Copy"; }, 1600);
  });
}

// The Prompts and skills page's commit label: the commit of main the deploy
// published, from the commit.txt it writes. Without one (e.g. served locally),
// the label keeps its generic text.
const commitLabel = document.querySelector("[data-commit]");
if (commitLabel) {
  fetch("/commit.txt")
    .then((response) => (response.ok ? response.text() : ""))
    .then((text) => {
      const commit = text.trim();
      if (!/^[0-9a-f]{40}$/.test(commit)) return;
      const code = (text) => Object.assign(document.createElement("code"), { textContent: text });
      const link = Object.assign(document.createElement("a"), { href: `https://github.com/JacobStephens2/thirdshift/commit/${commit}` });
      link.append(code(commit.slice(0, 7)));
      commitLabel.replaceChildren("commit ", link, " of ", code("main"));
    })
    .catch(() => {});
}
