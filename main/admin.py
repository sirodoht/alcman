from django.contrib import admin
from django.contrib.auth.admin import UserAdmin as DjUserAdmin

from main import models


@admin.register(models.User)
class UserAdmin(DjUserAdmin):
    list_display = (
        "id",
        "username",
        "email",
        "sync_ongoing",
        "synced_at",
        "date_joined",
        "last_login",
    )
    list_display_links = ("id", "username")

    fieldsets = (
        (
            None,
            {"fields": ("username", "password", "email", "sync_ongoing", "synced_at")},
        ),
        (
            "Permissions",
            {
                "fields": (
                    "is_active",
                    "is_staff",
                    "is_superuser",
                    "groups",
                    "user_permissions",
                )
            },
        ),
        ("Important dates", {"fields": ("last_login", "date_joined")}),
    )
    search_fields = ("username", "email")

    ordering = ["-id"]


@admin.register(models.Subscription)
class SubscriptionAdmin(admin.ModelAdmin):
    list_display = (
        "id",
        "title",
        "slug",
        "url",
        "user",
        "updated_at",
    )

    ordering = ["-id"]


@admin.register(models.Article)
class ArticleAdmin(admin.ModelAdmin):
    list_display = (
        "id",
        "url",
        "subscription",
        "published_at",
        "created_at",
    )

    ordering = ["-id"]
