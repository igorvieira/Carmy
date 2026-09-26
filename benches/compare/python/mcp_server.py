"""The same tool on MCPServer (formerly FastMCP), from the official Python MCP SDK 2.x."""
from typing import TypedDict

from mcp.server.mcpserver import MCPServer

from catalog import search


class Product(TypedDict):
    sku: str
    name: str
    price_cents: int


class SearchOutput(TypedDict):
    products: list[Product]


mcp = MCPServer("compare-py", log_level="WARNING")


@mcp.tool(description="Search the product catalog")
def search_products(query: str) -> SearchOutput:
    return search(query)


if __name__ == "__main__":
    mcp.run()
