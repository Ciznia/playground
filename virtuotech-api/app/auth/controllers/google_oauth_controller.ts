import type { HttpContext } from '@adonisjs/core/http'
import { ApiOperation, ApiResponse } from '@foadonis/openapi/decorators'
import GoogleOauthService from '#app/auth/services/google_oauth_service'

export default class GoogleOauthController {
  @ApiOperation({
    summary: 'Redirects the user to Google for authentication',
    tags: ['Authentication'],
  })
  @ApiResponse({
    status: 302,
    description: 'Redirects to Google OAuth consent screen',
  })
  async googleRedirect({ ally, session }: HttpContext) {
    return GoogleOauthService.googleRedirect(ally, session)
  }

  @ApiOperation({
    summary: 'Handles the Google OAuth callback',
    description: 'Retrieves the user information and create a new user if it does not exist',
    tags: ['Authentication'],
  })
  @ApiResponse({
    status: 200,
    description: 'User authenticated successfully',
  })
  async googleCallback({ ally }: HttpContext) {
    return GoogleOauthService.googleCallback(ally)
  }
}
