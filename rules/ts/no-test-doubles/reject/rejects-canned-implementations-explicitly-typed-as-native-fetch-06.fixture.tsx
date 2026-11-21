import { HttpResponse } from "msw"; const transport: typeof fetch = async () => HttpResponse.json({});
