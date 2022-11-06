from datetime import datetime
from urllib.request import urlopen

from django.contrib.auth.models import AbstractUser
from django.db import models
from django.utils import timezone
from django.utils.translation import gettext_lazy as _
from lxml import etree


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


class Subscription(models.Model):
    title = models.CharField(_("blog title"), max_length=200)
    url = models.URLField()
    user = models.ForeignKey(User, on_delete=models.CASCADE)
    updated_at = models.DateTimeField(default=timezone.now)

    def __str__(self):
        return self.url

    def get_data(self):
        with urlopen(self.url) as conn:
            xml_content = conn.read()
        root = etree.fromstring(xml_content)
        blog_title = root.find("channel").find("title").text
        article_list = []
        for child in root.find("channel"):
            if child.tag != "item":
                continue
            pub_date = child.find("pubDate").text
            published_at = datetime.strptime(pub_date, "%a, %d %b %Y %H:%M:%S %z")
            article_list.push(
                {
                    "title": child.find("title").text,
                    "url": child.find("link").text,
                    "published_at": published_at,
                    "body": child.find("description").text,
                }
            )
        return {
            "title": blog_title,
            "article_list": article_list,
        }


class Article(models.Model):
    subscription = models.ForeignKey(Subscription, on_delete=models.CASCADE)
    url = models.URLField()
    title = models.CharField(_("blog title"), max_length=300)
    body = models.TextField()
    published_at = models.DateField()
    created_at = models.DateTimeField(auto_now_add=True)

    def __str__(self):
        return self.url
