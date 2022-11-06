from django.contrib.auth.models import AbstractUser
from django.db import models
from django.utils.translation import gettext_lazy as _


class User(AbstractUser):
    email = models.EmailField(_("email address"))
    first_name = None
    last_name = None
    full_name = models.CharField(_("full name"), max_length=150, blank=True)
    email = models.EmailField(_("email address"), unique=True)

    class Meta:
        ordering = ["-id"]

    def __str__(self):
        return self.username
