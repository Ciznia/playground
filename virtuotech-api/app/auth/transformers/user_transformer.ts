import type User from '#app/auth/models/user'
import { BaseTransformer } from '@adonisjs/core/transformers'

export default class UserTransformer extends BaseTransformer<User> {
  toObject() {
    return this.pick(this.resource, [
      'id',
      'givenName',
      'familyName',
      'createdAt',
      'updatedAt',
    ])
  }
}
