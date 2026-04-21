import { test } from '@japa/runner'

test.group('check "Hello" root route', () => {
  test('GET root returns 200', async ({ client }) => {
    const response = await client.get('/')

    response.assertStatus(200)
    response.assertBody({
      hello: 'broken',
    })
  })
})
