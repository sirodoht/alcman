from unittest.mock import patch

from django.test import TestCase
from django.urls import reverse

from main import models, process_rss


class IndexTestCase(TestCase):
    def test_get(self):
        response = self.client.get(reverse("index"))
        self.assertEqual(response.status_code, 200)


class ProcessRSSTestCase(TestCase):
    def setUp(self):
        self.user = models.User.objects.create(username="alice")
        self.client.force_login(self.user)
        self.subscription = models.Subscription.objects.create(
            user=self.user, url="https://nutcroft.com/rss/"
        )

    @patch("main.process_rss.fetch")
    def test_fetch_and_save(self, mock_fetch):
        with open("main/testdata/rss.xml", "r") as f:
            mock_fetch.return_value = f.read().encode()
        process_rss.save_articles(self.subscription)
        self.assertEqual(
            models.Subscription.objects.get(id=self.subscription.id).title, "nutcroft"
        )
        self.assertEqual(models.Article.objects.all().count(), 62)
