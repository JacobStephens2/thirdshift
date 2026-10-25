const { rename, writeFile } = require('node:fs/promises');

// The recorder replaces Command records and Stop requests by rename, so the
// Console never reads one half-written. A fixture rewritten in place while a
// page is open can be read truncated: that push omits its row and the Inbox
// drops the row's selection and focus (#1223).
async function replace(path, text) {
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, text);
  await rename(temporary, path);
}

module.exports = { replace };
