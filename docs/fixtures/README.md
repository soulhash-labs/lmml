# Renderer Fixtures

Renderer fixtures are JSON arrays consumed by:

```text
lmml model verify-template \
  --endpoint http://127.0.0.1:1200 \
  --binary /path/to/llama-server \
  --model /path/to/model.gguf \
  --fixtures fixtures.json \
  --json
```

Each fixture must contain:

```json
[
  {
    "case_id": "system-user-basic",
    "messages": [
      {"role": "system", "content": "You are concise."},
      {"role": "user", "content": "Hello"}
    ],
    "rendered_prompt_hash": "<64 lowercase hex SHA-256>",
    "token_ids": [1, 2, 3]
  }
]
```

`rendered_prompt_hash` is the SHA-256 of the exact UTF-8 bytes returned by
llama.cpp `/apply-template`. `token_ids` are the exact IDs returned by the
same server's `/tokenize` endpoint for those rendered bytes. Fixtures must be
generated and reviewed against the exact binary, tokenizer, model, and launch
configuration intended for approval. A Python Transformers rendering result
cannot satisfy this check.

The verifier limits fixture count to 128, rendered output to 1 MiB per case,
and each renderer/tokenizer request to 60 seconds. Unsupported or malformed
fixture data fails closed.
