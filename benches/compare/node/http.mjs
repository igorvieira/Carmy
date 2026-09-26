// The same tool behind a plain Express handler.
import express from 'express';
import { search } from './catalog.mjs';

const [host, port] = (process.env.CARMY_ADDR ?? '127.0.0.1:3000').split(':');
const app = express();
app.use(express.json());
app.post('/agent/execute', (req, res) => {
  const { tool, arguments: args } = req.body ?? {};
  if (tool !== 'search_products' || typeof args?.query !== 'string') {
    return res.status(400).json({ error: 'invalid request' });
  }
  res.json({ status: 'completed', data: search(args.query) });
});
app.listen(Number(port), host);
