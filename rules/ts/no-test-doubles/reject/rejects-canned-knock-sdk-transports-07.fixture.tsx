import Client from "@knocklabs/node"; const Alias = Client; const fetcher = async () => new Response("ok"); const options = { fetch: fetcher }; new Alias(options);
