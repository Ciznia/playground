# Comment contribuer

## Convention de conduite

Suivez les consignes indiquées dans [ARCHITECTURE.md](./ARCHITECTURE.md)

## Serveur de développement

1. Aller dans le bon dossier

```sh
cd virtuotech-api
```

2. Installer les dependances

```sh
npm i
```

3. Configuration du .env

- Valeur initiale du .env

```sh
cp .env.example .env
node ace generate:key
```

- initialiser les secrets oauth google: [la documentation pour](https://developers.google.com/identity/protocols/oauth2)

4. lancer le serveur de dev

```sh
node ace serve --hmr
```
