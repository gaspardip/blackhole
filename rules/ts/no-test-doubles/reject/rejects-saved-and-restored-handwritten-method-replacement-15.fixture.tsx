import { client } from './client'; const original = client.send; client.send = () => false; client.send = original;
