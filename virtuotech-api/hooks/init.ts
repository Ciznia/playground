import { hooks } from '@adonisjs/core/app'

export default hooks.init((_parent, _hooksManager, indexGenerator) => {
  indexGenerator.add('controllers', {
    source: './app',
    importAlias: '#app',

    as: (vfs, buffer, _config, helpers) => {
      /**
       * asList() returns:
       *   key   → relative path WITHOUT extension  e.g. 'modules/mail/controller/mail_controller'
       *   value → absolute file path               e.g. '/project/app/modules/mail/controller/mail_controller.ts'
       */
      const controllerDirName: string = 'controllers'
      const files = vfs.asList()
      const tree: Record<string, unknown> = {}

      for (const [relativePath, absolutePath] of Object.entries(files)) {
        // Only process files sitting inside a directory named 'controllers'
        // and whose filename ends with _controller
        if (
          !relativePath.includes(controllerDirName + '/') ||
          !relativePath.endsWith('_controller')
        ) {
          continue
        }
        // helpers.toImportPath converts absolute path → aliased import
        // e.g. '/project/app/modules/mail/controller/mail_controller.ts'
        //    → '#controllers/modules/mail/controller/mail_controller'
        const importPath = helpers.toImportPath(absolutePath)

        const parts = relativePath.split('/')

        // Strip the intermediate 'controllers' directory name from the key path
        // so we get 'modules.mail.Mail' instead of 'modules.mail.controllers.Mail'
        const withoutControllerDir = parts.filter((p) => p !== controllerDirName)

        // 'mail_controller' → 'Mail', 'token_create_controller' → 'TokenCreate'
        const fileName = withoutControllerDir.at(-1)!
        const exportKey = fileName
          .replace(/_controller$/, '')
          .split('_')
          .map((s) => s.charAt(0).toUpperCase() + s.slice(1))
          .join('')

        // Everything before the filename becomes the nesting key path
        const keyPath = withoutControllerDir.slice(0, -1)

        // Walk/build the nested tree object
        let node = tree as Record<string, unknown>
        for (const key of keyPath) {
          if (!node[key]) node[key] = {}
          node = node[key] as Record<string, unknown>
        }
        node[exportKey] = importPath
      }

      function serialize(node: Record<string, unknown>, depth: number): string {
        const pad = '  '.repeat(depth)
        const inner = '  '.repeat(depth + 1)
        const lines = Object.entries(node).map(([key, value]) => {
          if (typeof value === 'string') {
            return `${inner}${key}: () => import('${value}'),`
          }
          return `${inner}${key}: ${serialize(value as Record<string, unknown>, depth + 1)},`
        })
        return `{\n${lines.join('\n')}\n${pad}}`
      }

      buffer.writeLine(`export const controllers = ${serialize(tree, 0)}`)
    },

    output: './.adonisjs/server/controllers.ts',
  })
})
