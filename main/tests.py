from django.test import TestCase
from django.urls import reverse


class IndexTestCase(TestCase):
    def test_get(self):
        response = self.client.get(reverse("index"))
        self.assertEqual(response.status_code, 200)