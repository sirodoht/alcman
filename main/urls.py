from django.urls import path

from main import views

urlpatterns = [
    path("", views.index, name="index"),
    path("blog/<slug:subscription_slug>/", views.subscription, name="subscription"),
    path("post/<slug:article_slug>/", views.article, name="article"),
    path("new/", views.SubscriptionCreateView.as_view(), name="new"),
]
