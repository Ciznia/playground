import User from '#app/auth/models/user'
import { loginValidator } from '#app/auth/validators/user'
import type { HttpContext } from '@adonisjs/core/http'
import UserTransformer from '#app/auth/transformers/user_transformer'
import { ApiOperation, ApiBody, ApiResponse } from '@foadonis/openapi/decorators'

export default class AccessTokenController {
  @ApiOperation({
    summary: 'Create a new access token',
    description: 'Generates a new access token for an existing user.',
  })
  @ApiBody({
    description: 'User login credentials',
    required: true,
    type: () => loginValidator.schema,
  })
  @ApiResponse({
    status: 200,
    description: 'Access token generated successfully',
    // Same issue with typing as in new_account_controller.ts
  })
  async store({ request, serialize }: HttpContext) {
    const { email, password } = await request.validateUsing(loginValidator)

    const user = await User.verifyCredentials(email, password)
    const token = await User.accessTokens.create(user)

    return serialize({
      user: UserTransformer.transform(user),
      token: token.value!.release(),
    })
  }

  @ApiOperation({
    summary: 'Revoke the current access token',
    description: 'Logs out the user by revoking their current access token.',
  })
  @ApiResponse({
    status: 200,
    description: 'User logged out successfully',
    schema: {
      type: 'object',
      properties: {
        message: {
          type: 'string',
          example: 'Logged out successfully',
        },
      },
    },
  })

  async destroy({ auth }: HttpContext) {
    const user = auth.getUserOrFail()
    if (user.currentAccessToken) {
      await User.accessTokens.delete(user, user.currentAccessToken.identifier)
    }

    return {
      message: 'Logged out successfully',
    }
  }
}
