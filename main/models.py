from django.contrib.auth.models import AbstractUser
from django.db import models
from django.utils.translation import gettext_lazy as _


class User(AbstractUser):
    email = models.EmailField(_("Email address"), unique=True)
    first_name = None
    last_name = None
    sync_ongoing = models.BooleanField(default=False)
    synced_at = models.DateTimeField(null=True)

    class Meta:
        ordering = ["-id"]

    def __str__(self):
        return self.username


class Subscription(models.Model):
    title = models.CharField(_("Blog Title"), max_length=200, blank=True, null=True)
    slug = models.SlugField(unique=True)
    url = models.URLField(_("Blog URL"))
    user = models.ForeignKey(User, on_delete=models.CASCADE)
    updated_at = models.DateTimeField(null=True)

    class Meta:
        ordering = ["-title"]

    def __str__(self):
        return self.url


class Article(models.Model):
    subscription = models.ForeignKey(Subscription, on_delete=models.CASCADE)
    url = models.URLField(_("Blog Post URL"))
    title = models.CharField(_("Blog Post Title"), max_length=300)
    slug = models.SlugField(unique=True)
    body = models.TextField(_("Blog Post Content"))
    published_at = models.DateField()
    created_at = models.DateTimeField(auto_now_add=True)

    class Meta:
        ordering = ["-published_at"]

    def __str__(self):
        return self.url
