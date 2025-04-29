## Endpoints

### OPTIONS /*

#### Request

```
OPTIONS / HTTP/1.1
Origin: …
```

#### Response

```
HTTP/1.1 200 OK
Access-Control-Allow-Origin: … (should be limited to allowed backend)
Cache-Control: max-age=31536000, immutable
```

### POST /

#### Request

```
POST / HTTP/1.1
Content-Type: application/octet-stream (implied)
Content-Length: … (max 3 * 1024 ** 2)

<encrypted data>
```

#### Response

```
HTTP/1.1 200 (ignored) OK (ignored)
Access-Control-Allow-Origin: … (should be limited to allowed backend)
Content-Type: application/json
Content-Length: …

<body, see below>
```

#### Response body

```ts
type Response = {
    // on success
    id: string, // matches [a-zA-Z0-9_-]+
} & {
    // on failure
    // frustratingly, other error types are not handled,
    // so just throw plaintext at them (which is handled!)
    error_class: "RequestTooLargeError",
};
```

### GET /:id

#### Request

```
GET /… HTTP/1.1
```

#### Response (success)

```
HTTP/1.1 200 OK
Access-Control-Allow-Origin: … (should be limited to allowed backend)
Cache-Control: max-age=31536000, immutable
Content-Type: application/octet-stream
Content-Length: …

<encrypted data>
```

#### Response (error)

any non-OK status code
