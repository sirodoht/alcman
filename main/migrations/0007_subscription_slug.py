# Generated by Django 4.1.3 on 2022-11-07 00:56

from django.db import migrations, models


class Migration(migrations.Migration):

    dependencies = [
        ("main", "0006_alter_article_options_alter_article_body_and_more"),
    ]

    operations = [
        migrations.AddField(
            model_name="subscription",
            name="slug",
            field=models.SlugField(default="sample-slug"),
            preserve_default=False,
        ),
    ]