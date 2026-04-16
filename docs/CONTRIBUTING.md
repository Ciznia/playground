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

- sur l'hôte:

```sh
node ace serve --hmr
```

- avec docker:

1. build l'image

```sh
docker build -t virtuoos-backend-v2 -f ./Dockerfile.dev .
```

2. lancer le serveur

- sans hot reload

```sh
# -p is for port bridging -e is to set HOST to 0.0.0.0 to allow the dockerfile to listen on any incoming adress, like the host one
docker run --env-file .env -p 3333:3333 -e HOST=0.0.0.0 virtuoos-backend-v2:latest
```

- avec hot reload sur powershell:

```sh
# -v is to mount volume, here we use it to mound the whole project folder and avoid node modules conflict between host and container
docker run --env-file .env -p 3333:3333 -e HOST=0.0.0.0 -v ${PWD}:/app -v /app/node_modules virtuoos-backend-v2:latest
```

- avec hot reload sur linux

```sh
# -v is to mount volume, here we use it to mound the whole project folder and avoid node modules conflict between host and container
docker run --env-file .env -p 3333:3333 -e HOST=0.0.0.0 -v $PWD:/app -v /app/node_modules virtuoos-backend-v2:latest
```
