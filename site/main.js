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
