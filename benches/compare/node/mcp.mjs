// The same tool on the official TypeScript MCP SDK.
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { z } from 'zod';
import { search } from './catalog.mjs';

const server = new McpServer({ name: 'compare-ts', version: '1.0.0' });
const product = z.object({ sku: z.string(), name: z.string(), price_cents: z.number() });

server.registerTool(
  'search_products',
  {
    description: 'Search the product catalog',
    inputSchema: { query: z.string() },
    outputSchema: { products: z.array(product) },
  },
  async ({ query }) => {
    const out = search(query);
    return { content: [{ type: 'text', text: JSON.stringify(out) }], structuredContent: out };
  },
);

await server.connect(new StdioServerTransport());
