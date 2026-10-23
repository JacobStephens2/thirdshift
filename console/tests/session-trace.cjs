// Session-trace fixtures retain raw Harness events inside OTLP log records.
function record(raw) {
  return JSON.stringify({ resourceLogs: [{ scopeLogs: [{ logRecords: [{
    body: { stringValue: raw }
  }] }] }] }) + '\n';
}

module.exports = { record };
