export function redactRequest(req: Object & {headers: {Authorization: String}}) {
  return {
    ...req,
    headers: {
      ...req.headers,
      Authorization: req.headers.Authorization
        ? "Bearer <REDACTED>"
        : undefined,
    }
  }
}
