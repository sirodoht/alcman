# Generated by Django 4.1.3 on 2022-11-07 01:04

from django.db import migrations, models


class Migration(migrations.Migration):

    dependencies = [
        ("main", "0009_alter_article_options_alter_subscription_options"),
    ]

    operations = [
        migrations.AlterField(
            model_name="subscription",
            name="updated_at",
            field=models.DateTimeField(null=True),
        ),
    ]
