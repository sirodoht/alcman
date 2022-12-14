# Generated by Django 4.1.3 on 2022-11-07 01:07

from django.db import migrations, models


class Migration(migrations.Migration):

    dependencies = [
        ("main", "0010_alter_subscription_updated_at"),
    ]

    operations = [
        migrations.RemoveField(
            model_name="user",
            name="full_name",
        ),
        migrations.AddField(
            model_name="user",
            name="sync_ongoing",
            field=models.BooleanField(default=False),
        ),
        migrations.AddField(
            model_name="user",
            name="synced_at",
            field=models.DateTimeField(null=True),
        ),
    ]
