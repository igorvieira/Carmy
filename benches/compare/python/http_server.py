"""The same tool behind a plain FastAPI handler, served by uvicorn."""
import os

import uvicorn
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

from catalog import search

app = FastAPI()


class Request(BaseModel):
    tool: str
    arguments: dict


@app.post("/agent/execute")
def execute(request: Request):
    query = request.arguments.get("query")
    if request.tool != "search_products" or not isinstance(query, str):
        raise HTTPException(status_code=400, detail="invalid request")
    return {"status": "completed", "data": search(query)}


if __name__ == "__main__":
    host, port = os.environ.get("CARMY_ADDR", "127.0.0.1:3000").split(":")
    uvicorn.run(app, host=host, port=int(port), log_level="warning", access_log=False)
