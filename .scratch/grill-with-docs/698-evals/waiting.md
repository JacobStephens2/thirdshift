# Waiting on research

Part of the [#698 grilling](README.md).

- **Live checks before building** (Q6.3–Q6.6):
  - whether each Harness logs in, refreshes, and writes its refresh back inside bwrap, with only its login file bound, and whether Claude writes its credentials file in place (Q6.4 (b));
  - whether each Harness works through `HTTPS_PROXY` over a Unix-socket relay, and which hosts each provider needs, for each Harness's allowlist;
  - how to turn off each Harness's web search, and whether it runs at the provider, past the proxy;
  - how to turn each Harness's memory off, unless a fresh home per Trial does it;
  - whether interruption still reaches the Harness through bwrap's PID namespace;
  - a Trial's real memory beside two building Runs, which sets Q6.6's cap.
- **If a viewer is added later** (Q5.5): its memory beside two building Runs on the `thirdshift` host, and its Content-Security-Policy tried against the real page.
- **Plan-limit signals** (Q3.7): what each Harness reports before it reaches its limit. thirdshift reads only Claude's usage-limit errors today. The Experiment Spec needs this, and a question follows only if some Harness gives no warning.
- The landscape note's other live checks (§7) still wait until a question needs one.
