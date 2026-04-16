export default {
  extends: ['@commitlint/config-conventional'],
  formatter: '@commitlint/format',
  plugins: [
    {
      rules: {
        'scope-jira-id':
        ({scope}) => {
          const regex = /^\[VS-\d+\]/;
          if (!scope) {
            return [
              false,
              'Scope is required and must match "[VS-<number>] <description>"',
            ];
          }

          return [
            regex.test(scope),
            'Scope must match the pattern "[VS-<number>] <description>" (e.g., "[VS-123] add-login")',
          ];
        },

      }
    }
  ],
  rules: {
    'header-max-length': [2, 'always', 72],
    'header-trim': [2, 'always'],
    'scope-empty': [2, 'never'],
    'scope-jira-id': [2, 'always'],
  },
};
