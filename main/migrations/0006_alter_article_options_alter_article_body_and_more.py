# Generated by Django 4.1.3 on 2022-11-07 00:56

from django.db import migrations, models


class Migration(migrations.Migration):

    dependencies = [
        ("main", "0005_alter_subscription_title"),
    ]

    operations = [
        migrations.AlterModelOptions(
            name="article",
            options={"ordering": ["-title"]},
        ),
        migrations.AlterField(
            model_name="article",
            name="body",
            field=models.TextField(verbose_name="Blog Post Content"),
        ),
        migrations.AlterField(
            model_name="article",
            name="title",
            field=models.CharField(max_length=300, verbose_name="Blog Post Title"),
        ),
        migrations.AlterField(
            model_name="article",
            name="url",
            field=models.URLField(verbose_name="Blog Post URL"),
        ),
        migrations.AlterField(
            model_name="subscription",
            name="title",
            field=models.CharField(
                blank=True, max_length=200, null=True, verbose_name="Blog Title"
            ),
        ),
        migrations.AlterField(
            model_name="subscription",
            name="url",
            field=models.URLField(verbose_name="Blog URL"),
        ),
        migrations.AlterField(
            model_name="user",
            name="email",
            field=models.EmailField(
                max_length=254, unique=True, verbose_name="Email address"
            ),
        ),
        migrations.AlterField(
            model_name="user",
            name="full_name",
            field=models.CharField(
                blank=True, max_length=150, verbose_name="Full name"
            ),
        ),
    ]