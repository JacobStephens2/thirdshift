#!/usr/bin/env node
'use strict';

// Serve the static site directory on an OS-assigned ephemeral 127.0.0.1 port
// and print the bound origin to stdout once ready.
const { createServer } = require('node:http');
const { readFile, realpath } = require('node:fs/promises');
const { join, resolve, sep } = require('node:path');

const siteDir = resolve(process.argv[2] ?? join(__dirname, '../../../..', 'site'));

const TYPES = {
  css: 'text/css; charset=utf-8',
  html: 'text/html; charset=utf-8',
  jpg: 'image/jpeg',
  js: 'text/javascript; charset=utf-8',
  png: 'image/png',
  svg: 'image/svg+xml',
  txt: 'text/plain; charset=utf-8',
  woff2: 'font/woff2',
  xml: 'application/xml; charset=utf-8',
};

async function main() {
  const root = await realpath(siteDir);
  const server = createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://127.0.0.1').pathname);
      const relative = pathname.endsWith('/') ? `${pathname.slice(1)}index.html` : pathname.slice(1);
      const candidate = resolve(root, relative);
      if (candidate !== root && !candidate.startsWith(root + sep)) {
        response.writeHead(403).end();
        return;
      }
      const target = await realpath(candidate);
      if (target !== root && !target.startsWith(root + sep)) {
        response.writeHead(403).end();
        return;
      }
      const content = await readFile(target);
      const ext = target.split('.').pop();
      response.writeHead(200, { 'Content-Type': TYPES[ext] ?? 'application/octet-stream' }).end(content);
    } catch {
      response.writeHead(404).end();
    }
  });

  const close = () => server.close(() => process.exit(0));
  process.on('SIGTERM', close);
  process.on('SIGINT', close);

  await new Promise(resolveListen => server.listen(0, '127.0.0.1', resolveListen));
  process.stdout.write(`http://127.0.0.1:${server.address().port}\n`);
}

main().catch(error => {
  console.error(error);
  process.exit(1);
});
