import { Client } from 'db2-node';

const client = new Client({ host: 'localhost', database: 'testdb', user: 'test', password: 'test' });

export function run() {
  return client.query('VALUES 1');
}
