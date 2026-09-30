<!-- Generated from src/prompt.rs by src/prompts_page.rs; don't edit. Regenerate with UPDATE_PROMPTS=1 cargo test prompts_page -->

# Resume

Continues any session, once, that ended its turn while waiting on background work, which was killed with it.

```
Your background work (<background work>) was killed when your turn ended, because ending the turn ends the session.

Re-run whatever you were waiting on in the foreground, then finish your job.

You run headless: nobody is watching, and ending your turn ends the session. Run tests and other long commands in the foreground, raising the Bash timeout if needed. Never end your turn while a background task you depend on is still running: ending the turn kills it.
```
