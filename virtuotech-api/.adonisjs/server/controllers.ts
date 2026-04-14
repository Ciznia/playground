export const controllers = {
  auth: {
    AccessToken: () => import('#app/auth/controllers/access_token_controller'),
    GoogleOauth: () => import('#app/auth/controllers/google_oauth_controller'),
    NewAccount: () => import('#app/auth/controllers/new_account_controller'),
    Profile: () => import('#app/auth/controllers/profile_controller'),
  },
}

