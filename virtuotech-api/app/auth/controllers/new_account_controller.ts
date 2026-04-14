import User from '#app/auth/models/user'
import { signupValidator } from '#app/auth/validators/user'
import type { HttpContext } from '@adonisjs/core/http'
import UserTransformer from '#app/auth/transformers/user_transformer'
import { ApiOperation, ApiBody, ApiResponse } from '@foadonis/openapi/decorators'


export default class NewAccountController {
  @ApiOperation({
    summary: 'Create a new user account',
    description: 'Registers a new user and returns an access token.',
  })
  @ApiBody({
    description: 'User registration data',
    required: true,
    type: () => signupValidator.schema,
  })
  @ApiResponse({
    status: 201,
    description: 'User account created successfully',
    // It's possible to define custom types for the response but i didn't succeed to make
    // it work with this response structure that use a transformer internally
    // or we can just specify a static response schema exemple but it's verbose as hell
  })
  // It's also possible to use the ApiResponse decorator multiple times when we want to define
  // multiple response schemas for different status code but here the only possible response
  // other than 201 is the default validation error respone 422 or 500 which are global but not yet documented
  async store({ request, serialize }: HttpContext) {
    const { fullName, email, password } = await request.validateUsing(signupValidator)

    const user = await User.create({ fullName, email, password })
    const token = await User.accessTokens.create(user)

    return serialize({
      user: UserTransformer.transform(user),
      token: token.value!.release(),
    })
  }
}
