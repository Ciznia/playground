import { BaseSchema } from '@adonisjs/lucid/schema'

export default class extends BaseSchema {
  protected tableName = 'users'

  async up() {
    this.schema.alterTable(this.tableName, (table) => {
      table.dropColumn('password')
      table.dropColumn('full_name')
      table.string('family_name').notNullable()
      table.string('given_name').notNullable()
    })
  }

  async down() {
    this.schema.alterTable(this.tableName, (table) => {
      table.string('full_name').nullable()
      table.string('password').notNullable()
      table.dropColumn('family_name')
      table.dropColumn('given_name')
    })
  }
}
