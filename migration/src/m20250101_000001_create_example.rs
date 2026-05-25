use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Example::Table)
                    .if_not_exists()
                    .col(pk_auto(Example::Id))
                    .col(string(Example::Name))
                    .col(
                        timestamp_with_time_zone(Example::CreatedAt)
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Example::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Example {
    Table,
    Id,
    Name,
    CreatedAt,
}
