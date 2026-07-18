import fs from 'node:fs';
import path from 'node:path';
import type { Plugin } from 'vite';

/**
 * Persists the composition document to composition.json and exposes it to
 * external agents:
 *
 *   GET  /__composition  -> current document
 *   POST /__composition  -> replace document (validated as JSON, pretty-printed)
 *
 * Editing composition.json directly on disk also works: the watcher pushes
 * the new document into the running app over the Vite websocket.
 *
 * The in-app client sends `x-dualcut-client: app` with its POSTs so its own
 * saves are not echoed back to it.
 */
export default function compositionPersistence(): Plugin {
  const file = path.resolve(process.cwd(), 'composition.json');
  let lastWritten = '';

  const readFile = () => fs.readFileSync(file, 'utf8');

  return {
    name: 'dualcut-composition',
    configureServer(server) {
      server.middlewares.use('/__composition', (req, res) => {
        if (req.method === 'GET') {
          res.setHeader('content-type', 'application/json');
          if (!fs.existsSync(file)) {
            res.statusCode = 404;
            res.end('{"error":"no composition.json yet"}');
            return;
          }
          res.end(readFile());
          return;
        }
        if (req.method === 'POST') {
          let body = '';
          req.on('data', (chunk) => (body += chunk));
          req.on('end', () => {
            res.setHeader('content-type', 'application/json');
            let parsed: unknown;
            try {
              parsed = JSON.parse(body);
            } catch (err) {
              res.statusCode = 400;
              res.end(JSON.stringify({ error: `invalid JSON: ${(err as Error).message}` }));
              return;
            }
            const pretty = `${JSON.stringify(parsed, null, 2)}\n`;
            lastWritten = pretty;
            fs.writeFileSync(file, pretty);
            // POSTs from anything other than the app itself (agents, curl)
            // get pushed into the running editor immediately.
            if (req.headers['x-dualcut-client'] !== 'app') {
              server.ws.send({
                type: 'custom',
                event: 'dualcut:external-composition',
                data: parsed,
              });
            }
            res.end('{"ok":true}');
          });
          return;
        }
        res.statusCode = 405;
        res.end();
      });

      server.watcher.add(file);
      server.watcher.on('change', (changed) => {
        if (path.resolve(changed) !== file) return;
        let content: string;
        try {
          content = readFile();
        } catch {
          return;
        }
        if (content === lastWritten) return; // our own write echoing back
        try {
          server.ws.send({
            type: 'custom',
            event: 'dualcut:external-composition',
            data: JSON.parse(content),
          });
        } catch {
          // Half-written or invalid file; ignore until it parses.
        }
      });
    },
  };
}
