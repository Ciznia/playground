# 🏗️ Architecture Virtuoos Backend v2

## 📋 Table des matières

- [Vue d'ensemble](#vue-densemble)
- [Structure du projet](#structure-du-projet)
- [Principes de conception](#principes-de-conception)
- [Conventions de codage](#conventions-de-conduite)
- [Flux de développement](#flux-de-développement)

## Vue d'ensemble

Virtuoos Backend v2 est un projet suivant l'architecture de construite de `modular monolithique` avec **TypeScript** et **AdonisJS V7**. Le pattern **modular monolith** est un design d'architecture où chaque domaine métier est isolé dans sa propre librairie.

### Technologies principales

- **TypeScript**: Typage statique
- **AdonisJS**: Framework web/router
- **Node.js**: Runtime

## Structure du projet

```txt
virtuotech-api/
├── app/                               # Couche logique
│   ├── [module transversal]/
│   ├── modules/
│   │   ├── [module-a]
│   │   └── [module-b]
├── config/                            # Tous les fichiers en lien avec la configuration
│   ├── database.ts                    # Configuration de la db
│   ├── auth.ts                        # Configuration des tokens utilisés pour l'auth
│   ├── openapi.ts                     # Configuration du swagger
│   └── ally.ts                        # Configuration oauth
└── database/                          # ORM et migrations
```

### Chaque module contient

Rappel: les modules sont des dossiers soit suffisamment transversaux pour etre directement dans app soit dans app/modules/

```txt
./[module]/
├── controllers/              # Les controlleurs du modules
│   ├── [name]_controller.ts
├── models/                   # Modèles typés de db
├── transformers/             # Définit les champs retournés à l'utilisateur final
├── validators/               # Schéma de validation Vine pour les corps de requêtes
├── routes.ts                 # Enregistre les différents endpoints
└── README.md                 # Optionnel si le nom n'est pas suffisament clair pour expliquer
                                ce que le module gère en tant que business logic
```

## Principes de conception

### 1. **Centralisation de la configuration**

#### Environnement

- aller dans la [configuration de l'environnement](../virtuotech-api\start\env.ts)
- ajoutez la clé correctement typer

### 2. **Isolement des modules**

Chaque module doit: Avoir son propre router publique via `route.ts`

### 3. **Dépendances unidirectionnelles**

```txt
app/auth → app/core (OK)
app/auth → app/auth (OK - self)
app/auth → app/modules/stock (Interdit - coupling)
```

## Conventions de conduite

### Nommage

- **Fichiers**: `kebab-case` (ex: `access_token_controller.ts`)
- **Dossiers**: `kebab-case` (ex: `app/module/mail`)
- **Fonctions/variables**: `camelCase`
- **Types/Interfaces**: `PascalCase`
- **Constantes**: `UPPER_SNAKE_CASE`

### Exports

```typescript
// libs/stock/src/lib/stock.routes.ts
export function registerStockRoutes(version: 'v1' | 'v2') {
  // ...
}

// libs/stock/src/index.ts
export { registerStockRoutes } from './lib/stock.routes'
```

### Documentation

Toujours documenter les endpoints avec [@foadonis/openapi](https://friendsofadonis.com/docs/openapi/getting-started)

## Flux de développement

### Créer un nouveau module SaaS

1. Créer la structure:

```bash
mkdir -p app/modules/[nom-module]/{controllers,models,transformers,validators}
touch app/modules/[nom-module]/routes.ts
```

2. Créer `controllers/[name]_controller.ts`:

```typescript
import type { HttpContext } from '@adonisjs/core/http'

export default class ProfileController {
}

```

3. Créer `routes.ts`:

```typescript
import { middleware } from '#start/kernel'
import router from '@adonisjs/core/services/router'
import { controllers } from '#generated/controllers'

router
  .group(() => {
  })
  .prefix('/v2/[prefix]')

```

### Ajouter une nouvelle route

1. **Créer la fonction dans le contrôleur adéquat** dans `app/[module || modules/module]/controllers/[name]_controller.ts`

```typescript
// app/auth/controllers/profile_controller.ts
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

```

2. **Enregistrer dans le routes.ts du module**

```typescript
// app/auth/routes.ts

import { middleware } from '#start/kernel'
import router from '@adonisjs/core/services/router'
import { controllers } from '#generated/controllers'

router
  .group(() => {
    router
      .group(() => {
        router.get('/profile', [controllers.auth.Profile, 'show'])
      })
      .prefix('account')
      .as('profile')
      .use(middleware.auth())
  })
  .prefix('/v2')

```

## Bonnes pratiques

### À faire

- Utiliser les types TypeScript au maximum
- Centraliser la configuration
- Découpler les modules
- Documenter le code public
- Tester les routes
- Valider les requêtes utilisateur

### À éviter

- Importer entre modules SaaS
- Routes sans version
- Code sans type
- Dépendances circulaires

## Ressources

- [AdonisJS Documentation](https://docs.adonisjs.com)
- [OpenAPI plugin documentation](https://friendsofadonis.com/docs/openapi/getting-started)
- [TypeScript Handbook](https://www.typescriptlang.org/docs)
