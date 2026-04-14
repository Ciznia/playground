import UserTransformer from '#app/auth/transformers/user_transformer'
import type { HttpContext } from '@adonisjs/core/http'
import { ApiOperation, ApiResponse } from '@foadonis/openapi/decorators'

export default class ProfileController {
  @ApiOperation({
    summary: 'Get authenticated user profile',
    description: 'Returns the profile information of the currently authenticated user.',
  })
  @ApiResponse({
    status: 200,
    description: 'User profile retrieved successfully',
  })
  async show({ auth, serialize }: HttpContext) {
    return serialize(UserTransformer.transform(auth.getUserOrFail()))
  }
}
